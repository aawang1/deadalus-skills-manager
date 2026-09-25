import { useState } from "react";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AgentComposer } from "./AgentComposer";
import { SkillGraph } from "./SkillGraph";
import type { InstalledSkill, SkillGraphSnapshot } from "../types";

const mocks = vi.hoisted(() => ({ copy: vi.fn().mockResolvedValue(true) }));
vi.mock("./DisplaySummary", () => ({ DisplaySummary: ({ text }: { text?: string }) => <>{text}</> }));
vi.mock("./CreateCategoryDialog", () => ({
  CreateCategoryDialog: ({ fixedSkillIds }: { fixedSkillIds: string[] }) => <div data-testid="category-members">{fixedSkillIds.join(",")}</div>,
}));
vi.mock("../api", () => ({ api: {
  recommendAgentSkills: vi.fn(async () => ({
    viewId: "all", summary: "Recommended", warnings: [],
    results: [{ skillId: "a", score: 0.8 }],
  })),
  getSkillGraph: vi.fn(async () => graph),
} }));

const skills: InstalledSkill[] = ["a", "b"].map((id) => ({
  skillId: id, name: id === "a" ? "Alpha" : "Beta", description: id,
  isBuiltIn: false, enabledAgents: [], disabledAgents: [],
  path: id, sourcePath: id, scope: "library", inLibrary: true, backupSuppressed: false,
}));
const graph = {
  graphVersion: "selection-test", layoutVersion: "selection-test", profileId: "p", viewId: "all",
  excludedUnreadyCount: 0, excludedUnconnectedCount: 0, clusters: [], proximities: [],
  nodes: skills.map((skill) => ({
    ...skill, path: "", clusterId: null, centrality: 0.5, superseded: false,
    disabled: false, classificationStatus: "ready", broadCategory: "", clusterCategory: "",
  })),
  edges: [{ edgeId: "ab", sourceSkillId: "a", targetSkillId: "b", similarity: 0.8,
    nearestFallback: false, relations: [] }],
} as unknown as SkillGraphSnapshot;

function Harness() {
  const [ids, setIds] = useState<string[]>();
  return <>
    <SkillGraph activeView="all" isNative refreshKey={0} highlightSkillIds={ids ?? []}
      onToggleSkill={ids === undefined ? undefined : (id) => setIds((current) =>
        current?.includes(id) ? current.filter((value) => value !== id) : [...(current ?? []), id])} />
    <AgentComposer windowId={1} activeView="all" skills={skills} selectedSkillIds={ids}
      onCopy={mocks.copy} onRecommendationChange={(_, view, next) => setIds(view ? next : undefined)} />
  </>;
}

describe("editable recommendation selection", () => {
  it("synchronizes graph toggles, remove buttons, empty selection and copy/category members", async () => {
    const user = userEvent.setup();
    const { container } = render(<Harness />);
    await user.click(screen.getByRole("button", { name: "新建" }));
    await user.type(screen.getByRole("textbox", { name: "Agent 1 输入" }), "organize");
    await user.click(screen.getByRole("button", { name: "发送" }));
    const alpha = await screen.findByRole("button", { name: "Alpha Skill" });
    const beta = screen.getByRole("button", { name: "Beta Skill" });
    await screen.findByRole("button", { name: "取消选中 Alpha" });
    expect(screen.queryByText("80%")).not.toBeInTheDocument();
    expect(container.querySelector(".agent-recommendations__item-status")).toBeEmptyDOMElement();
    expect(alpha).toHaveClass("is-recommended");
    await user.dblClick(beta);
    expect(await screen.findByRole("button", { name: "取消选中 Beta" })).toBeInTheDocument();
    expect(screen.getByText("手动加入")).toBeInTheDocument();
    expect(container.querySelector(".skill-graph__edge.is-recommended")).toBeTruthy();
    await user.dblClick(alpha);
    expect(screen.queryByRole("button", { name: "取消选中 Alpha" })).not.toBeInTheDocument();
    expect(alpha).not.toHaveClass("is-recommended");
    await user.click(screen.getByRole("button", { name: "取消选中 Beta" }));
    expect(beta).not.toHaveClass("is-recommended");
    expect(screen.getByRole("button", { name: "创建新类别" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "增量复制" })).toBeDisabled();
    expect(screen.getByText("尚未选中 Skills，双击关系图中的节点可加入。")).toBeInTheDocument();
    await user.dblClick(beta);
    expect(beta).toHaveAttribute("aria-pressed", "true");
    await user.click(screen.getByRole("button", { name: "增量复制" }));
    const dialog = screen.getByRole("dialog", { name: "复制推荐 Skills" });
    await user.selectOptions(within(dialog).getByRole("combobox"), "agent:codex");
    await user.click(within(dialog).getByRole("button", { name: "确认复制" }));
    await waitFor(() => expect(mocks.copy).toHaveBeenCalledWith(["b"], "agent:codex", "incremental"));
    await user.click(screen.getByRole("button", { name: "创建新类别" }));
    expect(screen.getByTestId("category-members")).toHaveTextContent("b");
  });
});
