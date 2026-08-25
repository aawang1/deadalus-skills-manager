import { render, screen, waitFor } from "@testing-library/react";
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
  profileId: "profile-a",
  viewId: "cursor" as const,
  excludedUnreadyCount: 0,
  excludedUnconnectedCount: 0,
  nodes: [
    {
      skillId: "a",
      name: "Review Skill",
      description: "Reviews a change.",
      path: "C:/skills/a",
      enabledAgents: ["cursor" as const],
    },
    {
      skillId: "b",
      name: "Test Skill",
      description: "Tests a change.",
      path: "C:/skills/b",
      enabledAgents: ["cursor" as const],
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
    expect(screen.getByText("功能相似（向量过阈）")).toBeInTheDocument();
    expect(screen.getByText("前置或依赖")).toBeInTheDocument();
    expect(container.querySelectorAll(".skill-graph__edge-hit")).toHaveLength(2);
  });
});
