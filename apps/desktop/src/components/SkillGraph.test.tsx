import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { SkillGraph } from "./SkillGraph";

const mocks = vi.hoisted(() => ({
  getSkillGraph: vi.fn(),
}));

vi.mock("../api", () => ({
  api: { getSkillGraph: mocks.getSkillGraph },
}));

const graph = {
  graphVersion: "graph-a",
  layoutVersion: "layout-a",
  profileId: "profile-a",
  viewId: "cursor" as const,
  excludedUnreadyCount: 0,
  excludedUnconnectedCount: 0,
  clusters: [
    {
      clusterId: "cluster-dev",
      name: "开发",
      summary: "开发与测试能力",
      memberSkillIds: ["a", "b"],
      coreSkillIds: ["a", "b"],
      peripheral: false,
      semanticStatus: "ready" as const,
    },
  ],
  proximities: [],
  nodes: [
    {
      skillId: "a",
      name: "Review Skill",
      description: "Reviews a change.",
      path: "C:/skills/a",
      enabledAgents: ["cursor" as const],
      disabled: false,
      clusterId: "cluster-dev",
      broadCategory: "开发工具",
      clusterCategory: "代码审查",
      centrality: 0.92,
      superseded: false,
      classificationStatus: "ready" as const,
    },
    {
      skillId: "b",
      name: "Test Skill",
      description: "Tests a change.",
      path: "C:/skills/b",
      enabledAgents: ["cursor" as const],
      disabled: false,
      clusterId: "cluster-dev",
      broadCategory: "开发工具",
      clusterCategory: "自动化测试",
      centrality: 0.88,
      superseded: false,
      classificationStatus: "ready" as const,
    },
  ],
  edges: [
    {
      edgeId: "edge-a-b",
      sourceSkillId: "a",
      targetSkillId: "b",
      similarity: 0.9,
      nearestFallback: false,
      relations: [
        {
          relationshipType: "similar_to" as const,
          vectorType: "overall_function" as const,
          state: "over_threshold",
          source: "vector_similarity" as const,
          evidence: {},
        },
        {
          relationshipType: "depends_on" as const,
          state: "human_confirmed",
          source: "stored" as const,
          evidence: {},
        },
      ],
    },
  ],
};

describe("SkillGraph", () => {
  it("preserves canvas panning on release and during later node spring-back", async () => {
    mocks.getSkillGraph.mockResolvedValue(graph);
    const frames: FrameRequestCallback[] = [];
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      frames.push(callback);
      return frames.length;
    });
    const user = userEvent.setup();
    render(<SkillGraph activeView="cursor" isNative refreshKey={0} />);
    const node = await screen.findByRole("button", { name: "Review Skill Skill" });
    const canvas = screen.getByRole("img", { name: "Cursor Skills 径向关系图" });
    vi.spyOn(canvas, "getBoundingClientRect").mockReturnValue({
      x: 0, y: 0, left: 0, top: 0, right: 1000, bottom: 800,
      width: 1000, height: 800, toJSON: () => ({}),
    });
    const layer = canvas.firstElementChild!;
    const initial = layer.getAttribute("transform");
    await user.pointer([
      { keys: "[MouseLeft>]", target: canvas, coords: { clientX: 100, clientY: 100 } },
      { target: canvas, coords: { clientX: 250, clientY: 180 } },
    ]);
    const panned = layer.getAttribute("transform");
    expect(panned).not.toBe(initial);
    await user.pointer({ keys: "[/MouseLeft]", target: canvas });
    expect(frames).toHaveLength(0);
    expect(layer.getAttribute("transform")).toBe(panned);
    await user.pointer([
      { keys: "[MouseLeft>]", target: node, coords: { clientX: 250, clientY: 180 } },
      { target: canvas, coords: { clientX: 280, clientY: 200 } },
      { keys: "[/MouseLeft]", target: canvas },
    ]);
    expect(frames.length).toBeGreaterThan(0);
    act(() => {
      for (let i = 0; frames.length && i < 100; i++) frames.shift()!(i * 16);
    });
    expect(frames).toHaveLength(0);
    expect(layer.getAttribute("transform")).toBe(panned);
  });

  it("opens node details on double click and closes outside", async () => {
    mocks.getSkillGraph.mockResolvedValue(graph);
    const user = userEvent.setup();
    render(<SkillGraph activeView="cursor" isNative refreshKey={0} />);

    const node = await screen.findByRole("button", { name: "Review Skill Skill" });
    await user.dblClick(node);
    expect(screen.getByText("Reviews a change.")).toBeInTheDocument();

    await user.click(
      screen.getByRole("img", { name: "Cursor Skills 径向关系图" }),
    );
    expect(screen.queryByText("Reviews a change.")).not.toBeInTheDocument();
  });

  it("expands one edge into relation details and parallel hit lines", async () => {
    mocks.getSkillGraph.mockResolvedValue(graph);
    const user = userEvent.setup();
    const { container } = render(
      <SkillGraph activeView="cursor" isNative refreshKey={0} />,
    );

    await waitFor(() =>
      expect(container.querySelector(".skill-graph__edge-hit")).toBeTruthy(),
    );
    await user.dblClick(container.querySelector(".skill-graph__edge-hit")!);

    expect(screen.getByText("具体关系")).toBeInTheDocument();
    expect(screen.getByText("功能相似")).toBeInTheDocument();
    expect(screen.queryByText(/向量过阈|超过可视化阈值|over_threshold/)).not.toBeInTheDocument();
    expect(screen.getByText("前置或依赖")).toBeInTheDocument();
    expect(container.querySelectorAll(".skill-graph__edge-hit")).toHaveLength(2);
  });

  it("opens cluster summary on double click", async () => {
    mocks.getSkillGraph.mockResolvedValue(graph);
    const user = userEvent.setup();
    render(<SkillGraph activeView="cursor" isNative refreshKey={0} />);

    const cluster = await screen.findByRole("button", { name: "开发 集群" });
    expect(cluster).toHaveAttribute("transform");
    expect(cluster.querySelector("circle")).toBeTruthy();
    expect(cluster.querySelector("text")).toHaveTextContent("开发");
    await user.dblClick(cluster);
    expect(screen.getByText("开发与测试能力")).toBeInTheDocument();
  });
});
