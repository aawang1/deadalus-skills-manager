import { describe, expect, it } from "vitest";
import type { SkillGraphEdge, SkillGraphNode } from "../types";
import {
  calculateSkillGraphLayout,
  graphViewport,
  parallelEdgeOffset,
} from "./skillGraphLayout";

const nodes: SkillGraphNode[] = [
  {
    skillId: "a",
    name: "A",
    path: "C:/a",
    enabledAgents: ["cursor"],
  },
  {
    skillId: "b",
    name: "B",
    path: "C:/b",
    enabledAgents: ["cursor"],
  },
  {
    skillId: "c",
    name: "C",
    path: "C:/c",
    enabledAgents: ["cursor"],
  },
];

const edges: SkillGraphEdge[] = [
  {
    edgeId: "a-b",
    sourceSkillId: "a",
    targetSkillId: "b",
    similarity: 0.92,
    nearestFallback: false,
    relations: [
      {
        relationshipType: "similar_to",
        vectorType: "overall_function",
        state: "over_threshold",
        source: "vector_similarity",
        evidence: {},
      },
    ],
  },
  {
    edgeId: "b-c",
    sourceSkillId: "b",
    targetSkillId: "c",
    similarity: 0.66,
    nearestFallback: true,
    relations: [
      {
        relationshipType: "similar_to",
        vectorType: "overall_function",
        state: "nearest_fallback",
        source: "vector_similarity",
        evidence: {},
      },
    ],
  },
];

describe("skill graph layout", () => {
  it("is deterministic and gives every node a finite position", () => {
    const first = calculateSkillGraphLayout(nodes, edges);
    const second = calculateSkillGraphLayout(nodes, edges);

    expect([...first.entries()]).toEqual([...second.entries()]);
    expect(
      [...first.values()].every(
        (point) => Number.isFinite(point.x) && Number.isFinite(point.y),
      ),
    ).toBe(true);
  });

  it("centers parallel edge offsets around the original line", () => {
    expect([0, 1, 2].map((index) => parallelEdgeOffset(index, 3))).toEqual([
      -7, 0, 7,
    ]);
    expect(parallelEdgeOffset(0, 1)).toBe(0);
  });

  it("keeps nodes apart from each other and from the virtual center", () => {
    const crowdedNodes = Array.from({ length: 36 }, (_, index) => ({
      skillId: `node-${index}`,
      name: `Node ${index}`,
      path: `C:/node-${index}`,
      enabledAgents: ["cursor" as const],
    }));
    const layout = calculateSkillGraphLayout(crowdedNodes, []);
    const points = [...layout.values()];

    expect(
      points.every(
        (point) =>
          Math.hypot(
            point.x - graphViewport.center.x,
            point.y - graphViewport.center.y,
          ) >= 81.9,
      ),
    ).toBe(true);
    for (let left = 0; left < points.length; left += 1) {
      for (let right = left + 1; right < points.length; right += 1) {
        expect(
          Math.hypot(
            points[left].x - points[right].x,
            points[left].y - points[right].y,
          ),
        ).toBeGreaterThanOrEqual(57.8);
      }
    }
  });
});
