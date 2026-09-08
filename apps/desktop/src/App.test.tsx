import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";

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
