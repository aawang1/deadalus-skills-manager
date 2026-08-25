import { describe, expect, it } from "vitest";
import type { SkillGraphSnapshot } from "../types";
import { calculateSkillGraphLayout, parallelEdgeOffset } from "./skillGraphLayout";

const graph: SkillGraphSnapshot = {
  graphVersion: "graph",
  layoutVersion: "layout",
  profileId: "profile",
  viewId: "cursor",
  excludedUnreadyCount: 0,
  excludedUnconnectedCount: 0,
  clusters: [{ clusterId: "cluster-a", name: "开发", summary: "开发能力", memberSkillIds: ["a", "b"], coreSkillIds: ["a", "b"], peripheral: false }],
  nodes: [
    { skillId: "a", name: "A", path: "C:/a", enabledAgents: ["cursor"], clusterId: "cluster-a", centrality: 0.94, superseded: false },
    { skillId: "b", name: "B", path: "C:/b", enabledAgents: ["cursor"], clusterId: "cluster-a", centrality: 0.9, superseded: false },
    { skillId: "c", name: "C", path: "C:/c", enabledAgents: ["cursor"], centrality: 0, superseded: false },
  ],
  edges: [{ edgeId: "a-b", sourceSkillId: "a", targetSkillId: "b", similarity: 0.92, nearestFallback: false, relations: [{ relationshipType: "similar_to", vectorType: "overall_function", state: "over_threshold", source: "vector_similarity", evidence: {} }] }],
  proximities: [],
};

describe("skill graph layout", () => {
  it("is deterministic and gives clusters and every node finite positions", () => {
    const first = calculateSkillGraphLayout(graph);
    const second = calculateSkillGraphLayout(graph);
    expect([...first.nodePoints.entries()]).toEqual([...second.nodePoints.entries()]);
    expect([...first.clusterPoints.entries()]).toEqual([...second.clusterPoints.entries()]);
    expect([...first.nodePoints.values()].every((point) => Number.isFinite(point.x) && Number.isFinite(point.y))).toBe(true);
  });

  it("keeps nodes from overlapping", () => {
    const layout = calculateSkillGraphLayout(graph);
    const points = [...layout.nodePoints.values()];
    for (let left = 0; left < points.length; left += 1) for (let right = left + 1; right < points.length; right += 1) {
      expect(Math.hypot(points[left].x - points[right].x, points[left].y - points[right].y)).toBeGreaterThanOrEqual(61.8);
    }
  });

  it("centers parallel edge offsets", () => {
    expect([0, 1, 2].map((index) => parallelEdgeOffset(index, 3))).toEqual([-7, 0, 7]);
    expect(parallelEdgeOffset(0, 1)).toBe(0);
  });
});
