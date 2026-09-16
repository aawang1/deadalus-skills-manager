import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import type { CustomSkillCategory } from "./types";

const mocks = vi.hoisted(() => ({
  getCanonicalSkillsSnapshot: vi.fn(),
  refreshCanonicalSkillsSnapshot: vi.fn(),
  semanticSearch: vi.fn(),
  getSkillGraph: vi.fn(),
  getSearchPreferences: vi.fn(),
  setIncludeDisabledSkills: vi.fn(),
  prepareSkillAction: vi.fn(),
  uninstallSkill: vi.fn(),
  disableSkillForAgent: vi.fn(),
  enableSkillForAgent: vi.fn(),
  restoreSkillBackup: vi.fn(),
  copyLibrarySkillToAgent: vi.fn(),
  listCustomSkillCategories: vi
    .fn<() => Promise<CustomSkillCategory[]>>()
    .mockResolvedValue([]),
  createCustomSkillCategory: vi.fn(),
  deleteCustomSkillCategory: vi.fn(),
  addSkillsToCustomCategory: vi.fn(),
  replaceCustomCategoryMembers: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("./api", () => ({
  isNativeRuntime: () => true,
  api: {
    getCanonicalSkillsSnapshot: mocks.getCanonicalSkillsSnapshot,
    refreshCanonicalSkillsSnapshot: mocks.refreshCanonicalSkillsSnapshot,
    semanticSearch: mocks.semanticSearch,
    getSkillGraph: mocks.getSkillGraph,
    getSearchPreferences: mocks.getSearchPreferences,
    setIncludeDisabledSkills: mocks.setIncludeDisabledSkills,
    prepareSkillAction: mocks.prepareSkillAction,
    uninstallSkill: mocks.uninstallSkill,
    disableSkillForAgent: mocks.disableSkillForAgent,
    enableSkillForAgent: mocks.enableSkillForAgent,
    restoreSkillBackup: mocks.restoreSkillBackup,
    copyLibrarySkillToAgent: mocks.copyLibrarySkillToAgent,
    listCustomSkillCategories: mocks.listCustomSkillCategories,
    createCustomSkillCategory: mocks.createCustomSkillCategory,
    deleteCustomSkillCategory: mocks.deleteCustomSkillCategory,
    addSkillsToCustomCategory: mocks.addSkillsToCustomCategory,
    replaceCustomCategoryMembers: mocks.replaceCustomCategoryMembers,
  },
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

describe("B3 assistant platform", () => {
  beforeEach(() => {
    mocks.getCanonicalSkillsSnapshot.mockResolvedValue({
      snapshotId: "snapshot-a",
      searchedPaths: [],
      warnings: [],
      skills: [],
    });
    mocks.refreshCanonicalSkillsSnapshot.mockResolvedValue({
      snapshotId: "snapshot-b",
      searchedPaths: [],
      warnings: [],
      skills: [],
    });
    mocks.semanticSearch.mockResolvedValue([]);
    mocks.getSearchPreferences.mockResolvedValue({ includeDisabledSkills: false });
    mocks.setIncludeDisabledSkills.mockImplementation((enabled: boolean) =>
      Promise.resolve({ includeDisabledSkills: enabled }),
    );
    mocks.getSkillGraph.mockResolvedValue({
      graphVersion: "graph-a",
      profileId: "profile-a",
      viewId: "all",
      nodes: [],
      edges: [],
      excludedUnreadyCount: 0,
      excludedUnconnectedCount: 0,
    });
    mocks.listen.mockResolvedValue(() => undefined);
    mocks.prepareSkillAction.mockReset();
    mocks.uninstallSkill.mockReset();
    mocks.disableSkillForAgent.mockReset();
    mocks.copyLibrarySkillToAgent.mockReset();
    mocks.replaceCustomCategoryMembers.mockReset();
  });

  it("shows a scoped second confirmation before deleting an all_skills backup", async () => {
    const snapshot = {
      snapshotId: "snapshot-skill",
      searchedPaths: [],
      warnings: [],
      skills: [{
        skillId: "skill-demo",
        name: "Demo Skill",
        path: "C:/agent/demo",
        sourcePath: "C:/agent",
        scope: "user",
        isBuiltIn: false,
        enabledAgents: ["codex"],
        disabledAgents: [],
        inLibrary: true,
        libraryPath: "C:/library/demo",
        backupSuppressed: false,
      }],
    };
    mocks.getCanonicalSkillsSnapshot.mockResolvedValue(snapshot);
    mocks.prepareSkillAction.mockResolvedValue({
      skillId: "skill-demo",
      skillName: "Demo Skill",
      viewId: "all",
      action: "uninstall",
      allowed: true,
      targetPaths: ["C:/library/demo"],
      affectedAgents: [],
      sharedInstallation: false,
      pluginOperation: false,
      symbolicLinkOnly: false,
      keepsLibraryCopy: false,
      keepsOtherAgents: true,
    });
    mocks.uninstallSkill.mockResolvedValue(snapshot);
    const user = userEvent.setup();
    render(<App />);
    await user.click(screen.getByRole("button", { name: "所有 Skills" }));
    await user.click(await screen.findByRole("button", { name: "删除后备副本 Demo Skill" }));
    expect(await screen.findByRole("heading", { name: "确认卸载" })).toBeInTheDocument();
    expect(screen.getByText(/停止在后续扫描中自动重新备份/)).toBeInTheDocument();
    expect(mocks.uninstallSkill).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "确认卸载" }));
    expect(mocks.uninstallSkill).toHaveBeenCalledWith("skill-demo", "all");
  });

  it("keeps the Codex disable confirmation actionable after a valid preflight", async () => {
    const snapshot = {
      snapshotId: "snapshot-codex-skill",
      searchedPaths: [],
      warnings: [],
      skills: [{
        skillId: "skill-docs",
        name: "OpenAI Docs",
        path: "C:/Users/demo/.codex/skills/.system/openai-docs",
        sourcePath: "C:/Users/demo/.codex/skills/.system",
        scope: "system",
        isBuiltIn: true,
        enabledAgents: ["codex"],
        disabledAgents: [],
        inLibrary: false,
        libraryPath: undefined,
        backupSuppressed: false,
      }],
    };
    mocks.getCanonicalSkillsSnapshot.mockResolvedValue(snapshot);
    mocks.prepareSkillAction.mockResolvedValue({
      skillId: "skill-docs",
      skillName: "OpenAI Docs",
      viewId: "codex",
      action: "disable",
      allowed: true,
      targetPaths: ["C:/Users/demo/.codex/skills/.system/openai-docs"],
      affectedAgents: ["codex"],
      sharedInstallation: false,
      pluginOperation: false,
      symbolicLinkOnly: false,
      keepsLibraryCopy: false,
      keepsOtherAgents: false,
    });
    mocks.disableSkillForAgent.mockResolvedValue(snapshot);
    const user = userEvent.setup();

    render(<App />);
    await user.click(screen.getByRole("button", { name: "Codex Skills" }));
    await user.click(await screen.findByRole("button", {
      name: "禁用当前 Agent 中的 OpenAI Docs",
    }));

    const confirm = await screen.findByRole("button", { name: "确认禁用" });
    expect(confirm).toBeEnabled();
    await user.click(confirm);
    expect(mocks.disableSkillForAgent).toHaveBeenCalledWith("skill-docs", "codex");
  });

  it("keeps the selected copy target stable and shows a dialog when copying fails", async () => {
    const snapshot = {
      snapshotId: "snapshot-portable-skill",
      searchedPaths: [],
      warnings: [],
      skills: [{
        skillId: "skill-portable",
        name: "Portable Skill",
        path: "C:/library/portable",
        sourcePath: "C:/library",
        scope: "user",
        isBuiltIn: false,
        enabledAgents: [],
        disabledAgents: [],
        inLibrary: true,
        libraryPath: "C:/library/portable",
        backupSuppressed: false,
      }],
    };
    mocks.getCanonicalSkillsSnapshot.mockResolvedValue(snapshot);
    mocks.copyLibrarySkillToAgent.mockRejectedValue("Skill 包与 Codex 不兼容");
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();

    render(<App />);
    await user.click(screen.getByRole("button", { name: "所有 Skills" }));
    await user.click(await screen.findByRole("button", {
      name: "展开 Portable Skill 的完整信息",
    }));
    const target = screen.getByRole("combobox", {
      name: "选择 Portable Skill 的安装目标",
    });
    await user.selectOptions(target, "codex");
    expect(target).toHaveValue("codex");
    await user.click(screen.getByRole("button", { name: "复制到 Agent" }));

    expect(mocks.copyLibrarySkillToAgent).toHaveBeenCalledWith(
      "C:/library/portable",
      "codex",
    );
    const alert = await screen.findByRole("alertdialog", { name: "复制失败" });
    expect(alert).toBeInTheDocument();
    expect(within(alert).getByText("Skill 包与 Codex 不兼容")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "所有 Skills" })).toBeInTheDocument();
  });

  it("starts collapsed, toggles from the circular control, and retains search state", async () => {
    const user = userEvent.setup();
    render(<App />);

    const platform = screen.getByRole("complementary", {
      name: "B3 语义搜索平台",
    });
    const expand = screen.getByRole("button", { name: "展开语义搜索" });
    expect(expand).toHaveAttribute("aria-expanded", "false");
    expect(platform.firstElementChild).toHaveAttribute("aria-hidden", "true");
    expect(screen.getByRole("main", { name: "A1 可视化工作区" })).toBeEmptyDOMElement();

    await user.click(expand);
    const collapse = screen.getByRole("button", { name: "收起语义搜索" });
    expect(collapse).toHaveAttribute("aria-expanded", "true");
    expect(platform.firstElementChild).toHaveAttribute("aria-hidden", "false");

    const searchbox = screen.getByRole("searchbox");
    await user.type(searchbox, "retained query");
    await user.click(collapse);
    expect(screen.getByRole("button", { name: "展开语义搜索" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    await user.click(screen.getByRole("button", { name: "展开语义搜索" }));
    expect(screen.getByRole("searchbox")).toHaveValue("retained query");
  });

  it("collapses B2 to an arrow strip while keeping the category selected", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: "所有 Skills" }));
    const category = screen.getByRole("button", { name: "所有 Skills" });
    expect(category).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("heading", { name: "所有 Skills" })).toBeInTheDocument();

    const collapse = screen.getByRole("button", { name: "收起 Skills 列表" });
    expect(collapse).toHaveAttribute("aria-expanded", "true");
    await user.click(collapse);

    expect(category).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByRole("heading", { name: "所有 Skills" })).not.toBeInTheDocument();
    const expand = screen.getByRole("button", { name: "展开 Skills 列表" });
    expect(expand).toHaveAttribute("aria-expanded", "false");

    await user.click(expand);
    expect(screen.getByRole("heading", { name: "所有 Skills" })).toBeInTheDocument();
  });
});

describe("B2 skills list collapse", () => {
  beforeEach(() => {
    mocks.getCanonicalSkillsSnapshot.mockResolvedValue({
      snapshotId: "snapshot-a",
      searchedPaths: [],
      warnings: [],
      skills: [],
    });
    mocks.refreshCanonicalSkillsSnapshot.mockResolvedValue({
      snapshotId: "snapshot-b",
      searchedPaths: [],
      warnings: [],
      skills: [],
    });
    mocks.semanticSearch.mockResolvedValue([]);
    mocks.getSkillGraph.mockResolvedValue({
      graphVersion: "graph-a",
      profileId: "profile-a",
      viewId: "all",
      nodes: [],
      edges: [],
      excludedUnreadyCount: 0,
      excludedUnconnectedCount: 0,
    });
    mocks.listen.mockResolvedValue(() => undefined);
  });

  it("collapses to a toggle strip while keeping the category selected", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: "所有 Skills" }));
    const category = screen.getByRole("button", { name: "所有 Skills" });
    expect(category).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("heading", { name: "所有 Skills" })).toBeInTheDocument();

    const collapse = screen.getByRole("button", { name: "收起 Skills 列表" });
    expect(collapse).toHaveAttribute("aria-expanded", "true");
    await user.click(collapse);

    expect(category).toHaveAttribute("aria-pressed", "true");
    expect(
      screen.queryByRole("heading", { name: "所有 Skills" }),
    ).not.toBeInTheDocument();
    const expand = screen.getByRole("button", { name: "展开 Skills 列表" });
    expect(expand).toHaveAttribute("aria-expanded", "false");

    await user.click(expand);
    expect(screen.getByRole("heading", { name: "所有 Skills" })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "收起 Skills 列表" }),
    ).toHaveAttribute("aria-expanded", "true");
  });

  it("keeps the collapsed strip when switching categories", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: "所有 Skills" }));
    await user.click(screen.getByRole("button", { name: "收起 Skills 列表" }));
    await user.click(screen.getByRole("button", { name: "Claude Code Skills" }));

    expect(screen.getByRole("button", { name: "所有 Skills" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    expect(
      screen.getByRole("button", { name: "Claude Code Skills" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "展开 Skills 列表" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(screen.queryByRole("heading", { name: "Claude Code" })).not.toBeInTheDocument();
  });
});

describe("left category groups", () => {
  beforeEach(() => {
    mocks.getCanonicalSkillsSnapshot.mockResolvedValue({
      snapshotId: "snapshot-groups",
      searchedPaths: [],
      warnings: [],
      skills: [],
    });
    mocks.getSkillGraph.mockResolvedValue({
      graphVersion: "graph-groups",
      layoutVersion: "layout-groups",
      profileId: "profile-a",
      viewId: "all",
      nodes: [],
      clusters: [],
      edges: [],
      proximities: [],
      excludedUnreadyCount: 0,
      excludedUnconnectedCount: 0,
    });
    mocks.listen.mockResolvedValue(() => undefined);
  });

  it("independently expands and collapses Agent, custom, and project categories", async () => {
    const user = userEvent.setup();
    render(<App />);

    const agentGroup = screen.getByRole("button", { name: "Agent 全局" });
    const customGroup = screen.getByRole("button", { name: "自定义类别" });
    const projectGroup = screen.getByRole("button", { name: "项目" });
    expect(agentGroup).toHaveAttribute("aria-expanded", "true");
    expect(customGroup).toHaveAttribute("aria-expanded", "true");
    expect(projectGroup).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("button", { name: "Claude Code Skills" })).toBeInTheDocument();
    expect(screen.getByText("暂无自定义类别")).toBeInTheDocument();
    expect(screen.getByText("暂无项目")).toBeInTheDocument();

    await user.click(agentGroup);
    expect(agentGroup).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("button", { name: "Claude Code Skills" })).not.toBeInTheDocument();

    await user.click(customGroup);
    await user.click(projectGroup);
    expect(screen.queryByText("暂无自定义类别")).not.toBeInTheDocument();
    expect(screen.queryByText("暂无项目")).not.toBeInTheDocument();
  });

  it("keeps an active Agent selected when its parent category is collapsed", async () => {
    const user = userEvent.setup();
    render(<App />);

    const codex = screen.getByRole("button", { name: "Codex Skills" });
    await user.click(codex);
    expect(codex).toHaveAttribute("aria-pressed", "true");
    await user.click(screen.getByRole("button", { name: "Agent 全局" }));
    expect(screen.queryByRole("button", { name: "Codex Skills" })).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Codex" })).toBeInTheDocument();
  });

  it("opens the custom category composer from the enabled create button", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: "新建自定义类别" }));
    expect(
      screen.getByRole("dialog", { name: "新建自定义类别" }),
    ).toBeInTheDocument();
  });

  it("renders persisted custom categories with their color and opens their view", async () => {
    mocks.listCustomSkillCategories.mockResolvedValueOnce([
      {
        categoryId: "category-visual",
        name: "视觉工具",
        color: "#de6ea8",
        description: "视觉设计相关 Skills",
        skillIds: [],
        createdAt: 1,
        updatedAt: 1,
      },
    ]);
    const user = userEvent.setup();
    render(<App />);

    const category = await screen.findByRole("button", {
      name: "视觉工具 Skills",
    });
    expect(category.querySelector(".category-button__dot")).toHaveStyle({
      backgroundColor: "#de6ea8",
    });
    await user.click(category);
    expect(screen.getByRole("heading", { name: "视觉工具" })).toBeInTheDocument();
    expect(mocks.getSkillGraph).toHaveBeenCalledWith("custom:category-visual");
  });

  it("deletes a virtual category only after right-click confirmation", async () => {
    mocks.listCustomSkillCategories.mockResolvedValueOnce([
      {
        categoryId: "category-delete",
        name: "待删除类别",
        color: "#57bd87",
        description: "测试删除",
        skillIds: [],
        createdAt: 1,
        updatedAt: 1,
      },
    ]);
    mocks.deleteCustomSkillCategory.mockResolvedValueOnce(true);
    const user = userEvent.setup();
    render(<App />);

    const category = await screen.findByRole("button", {
      name: "待删除类别 Skills",
    });
    fireEvent.contextMenu(category, { clientX: 120, clientY: 160 });
    await user.click(screen.getByRole("menuitem", { name: /删除该类别/ }));
    expect(
      screen.getByRole("alertdialog", { name: "删除自定义类别" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "确认删除" }));

    await waitFor(() =>
      expect(mocks.deleteCustomSkillCategory).toHaveBeenCalledWith(
        "category-delete",
      ),
    );
    expect(
      screen.queryByRole("button", { name: "待删除类别 Skills" }),
    ).not.toBeInTheDocument();
  });

  it("shows the category description and copies only missing members to another category", async () => {
    const alpha = {
      skillId: "skill-alpha",
      name: "Alpha",
      description: "Alpha description",
      path: "skills/alpha",
      sourcePath: "skills",
      scope: "user" as const,
      isBuiltIn: false,
      enabledAgents: ["codex" as const],
      disabledAgents: [],
      inLibrary: true,
      libraryPath: "library/alpha",
      backupSuppressed: false,
    };
    const beta = { ...alpha, skillId: "skill-beta", name: "Beta", path: "skills/beta" };
    mocks.getCanonicalSkillsSnapshot.mockResolvedValueOnce({
      snapshotId: "snapshot-copy",
      searchedPaths: [],
      warnings: [],
      skills: [alpha, beta],
    });
    mocks.listCustomSkillCategories.mockResolvedValueOnce([
      {
        categoryId: "source",
        name: "源类别",
        color: "#4f8cff",
        description: "用于测试的类别简介",
        skillIds: ["skill-alpha", "skill-beta"],
        createdAt: 1,
        updatedAt: 1,
      },
      {
        categoryId: "target",
        name: "目标类别",
        color: "#57bd87",
        description: "目标",
        skillIds: ["skill-alpha"],
        createdAt: 1,
        updatedAt: 1,
      },
    ]);
    mocks.addSkillsToCustomCategory.mockResolvedValueOnce({
      category: {
        categoryId: "target",
        name: "目标类别",
        color: "#57bd87",
        description: "目标",
        skillIds: ["skill-alpha", "skill-beta"],
        createdAt: 1,
        updatedAt: 2,
      },
      addedCount: 1,
      skippedCount: 0,
    });
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole("button", { name: "源类别 Skills" }));
    expect(screen.getByText("用于测试的类别简介")).toBeInTheDocument();
    await user.selectOptions(
      screen.getByLabelText("选择 源类别 的复制目标"),
      "custom:target",
    );
    await user.click(screen.getByRole("button", { name: "增量复制" }));

    await waitFor(() =>
      expect(mocks.addSkillsToCustomCategory).toHaveBeenCalledWith("target", [
        "skill-beta",
      ]),
    );
    confirm.mockRestore();
  });

  it("overwrites a virtual category with the exact source membership", async () => {
    const alpha = {
      skillId: "skill-alpha",
      name: "Alpha",
      description: "Alpha description",
      path: "skills/alpha",
      sourcePath: "skills",
      scope: "user" as const,
      isBuiltIn: false,
      enabledAgents: [] as const,
      disabledAgents: [],
      inLibrary: true,
      libraryPath: "library/alpha",
      backupSuppressed: false,
    };
    const source = {
      categoryId: "source-overwrite",
      name: "覆盖源",
      color: "#4f8cff",
      description: "源",
      skillIds: ["skill-alpha"],
      createdAt: 1,
      updatedAt: 1,
    };
    const target = {
      categoryId: "target-overwrite",
      name: "覆盖目标",
      color: "#57bd87",
      description: "目标",
      skillIds: ["skill-old"],
      createdAt: 1,
      updatedAt: 1,
    };
    mocks.getCanonicalSkillsSnapshot.mockResolvedValueOnce({
      snapshotId: "snapshot-overwrite",
      searchedPaths: [],
      warnings: [],
      skills: [alpha],
    });
    mocks.listCustomSkillCategories.mockResolvedValueOnce([source, target]);
    mocks.replaceCustomCategoryMembers.mockResolvedValueOnce({
      category: { ...target, skillIds: ["skill-alpha"], updatedAt: 2 },
      addedCount: 1,
      removedCount: 1,
      unchangedCount: 0,
    });
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole("button", { name: "覆盖源 Skills" }));
    await user.selectOptions(
      screen.getByLabelText("选择 覆盖源 的复制目标"),
      "custom:target-overwrite",
    );
    await user.selectOptions(
      screen.getByLabelText("选择 覆盖源 的复制方式"),
      "overwrite",
    );
    await user.click(screen.getByRole("button", { name: "覆盖目标类别" }));

    await waitFor(() =>
      expect(mocks.replaceCustomCategoryMembers).toHaveBeenCalledWith(
        "source-overwrite",
        "target-overwrite",
      ),
    );
    expect(mocks.addSkillsToCustomCategory).not.toHaveBeenCalled();
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("移除 1 个成员"));
    confirm.mockRestore();
  });

  it("overwrites an Agent and can disable shared removals instead of deleting them", async () => {
    const sourceSkill = {
      skillId: "skill-source",
      name: "Source Skill",
      description: "Source",
      path: "skills/source",
      sourcePath: "skills",
      scope: "user" as const,
      isBuiltIn: false,
      enabledAgents: [] as const,
      disabledAgents: [],
      inLibrary: true,
      libraryPath: "library/source",
      backupSuppressed: false,
    };
    const sharedSkill = {
      ...sourceSkill,
      skillId: "skill-shared",
      name: "Shared Skill",
      path: "skills/shared",
      enabledAgents: ["cursor", "codex"] as const,
      libraryPath: "library/shared",
    };
    mocks.getCanonicalSkillsSnapshot.mockResolvedValueOnce({
      snapshotId: "snapshot-agent-overwrite",
      searchedPaths: [],
      warnings: [],
      skills: [sourceSkill, sharedSkill],
    });
    mocks.listCustomSkillCategories.mockResolvedValueOnce([
      {
        categoryId: "source-agent-overwrite",
        name: "Agent 覆盖源",
        color: "#4f8cff",
        description: "源",
        skillIds: ["skill-source"],
        createdAt: 1,
        updatedAt: 1,
      },
    ]);
    mocks.prepareSkillAction.mockImplementation(
      (_skillId: string, viewId: string, action: "uninstall" | "disable") =>
        Promise.resolve({
          skillId: "skill-shared",
          skillName: "Shared Skill",
          viewId,
          action,
          allowed: true,
          targetPaths: ["skills/shared"],
          affectedAgents: ["cursor", "codex"],
          sharedInstallation: action === "uninstall",
          pluginOperation: false,
          symbolicLinkOnly: false,
          keepsLibraryCopy: true,
          keepsOtherAgents: true,
        }),
    );
    mocks.copyLibrarySkillToAgent.mockResolvedValueOnce(undefined);
    mocks.disableSkillForAgent.mockResolvedValueOnce({});
    const user = userEvent.setup();
    render(<App />);

    await user.click(
      await screen.findByRole("button", { name: "Agent 覆盖源 Skills" }),
    );
    await user.selectOptions(
      screen.getByLabelText("选择 Agent 覆盖源 的复制目标"),
      "agent:codex",
    );
    await user.selectOptions(
      screen.getByLabelText("选择 Agent 覆盖源 的复制方式"),
      "overwrite",
    );
    await user.click(screen.getByRole("button", { name: "覆盖目标 Agent" }));

    expect(
      await screen.findByRole("heading", { name: "覆盖 Codex" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/共享：cursor、codex/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "仍然删除" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "禁用风险项" }));

    await waitFor(() =>
      expect(mocks.copyLibrarySkillToAgent).toHaveBeenCalledWith(
        "library/source",
        "codex",
      ),
    );
    expect(mocks.disableSkillForAgent).toHaveBeenCalledWith(
      "skill-shared",
      "codex",
    );
    expect(mocks.uninstallSkill).not.toHaveBeenCalled();
  });

  it("adds an expanded All Skills item to a virtual category", async () => {
    const skill = {
      skillId: "skill-single",
      name: "Single Skill",
      description: "Single description",
      path: "skills/single",
      sourcePath: "skills",
      scope: "user" as const,
      isBuiltIn: false,
      enabledAgents: [],
      disabledAgents: [],
      inLibrary: true,
      libraryPath: "library/single",
      backupSuppressed: false,
    };
    const target = {
      categoryId: "target-single",
      name: "个人类别",
      color: "#8b7cff",
      description: "目标",
      skillIds: [],
      createdAt: 1,
      updatedAt: 1,
    };
    mocks.getCanonicalSkillsSnapshot.mockResolvedValueOnce({
      snapshotId: "snapshot-single",
      searchedPaths: [],
      warnings: [],
      skills: [skill],
    });
    mocks.listCustomSkillCategories.mockResolvedValueOnce([target]);
    mocks.addSkillsToCustomCategory.mockResolvedValueOnce({
      category: { ...target, skillIds: [skill.skillId], updatedAt: 2 },
      addedCount: 1,
      skippedCount: 0,
    });
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: "所有 Skills" }));
    await user.click(
      await screen.findByRole("button", {
        name: "展开 Single Skill 的完整信息",
      }),
    );
    expect(
      screen.getByLabelText("选择 Single Skill 的虚拟类别目标"),
    ).toHaveValue("target-single");
    await user.click(screen.getByRole("button", { name: "复制到类别" }));
    await waitFor(() =>
      expect(mocks.addSkillsToCustomCategory).toHaveBeenCalledWith(
        "target-single",
        ["skill-single"],
      ),
    );
  });
});
