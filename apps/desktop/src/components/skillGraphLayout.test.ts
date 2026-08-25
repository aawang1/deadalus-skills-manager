import { describe, expect, it } from "vitest";
import type { SkillGraphSnapshot } from "../types";
import { calculateSkillGraphLayout, parallelEdgeOffset, propagateClusterDragOffsets, propagateDragOffsets } from "./skillGraphLayout";

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

  it("propagates a dragged node through springs and prevents overlap", () => {
    const layout = calculateSkillGraphLayout(graph);
    const offsets = propagateDragOffsets(graph, layout, "a", { x: 140, y: 20 }, {});
    expect(Math.hypot(offsets.a.x, offsets.a.y)).toBeGreaterThan(100);
    expect(Math.hypot(offsets.b.x, offsets.b.y)).toBeGreaterThan(1);

    const positions = graph.nodes.map((node) => {
      const base = layout.nodePoints.get(node.skillId)!;
      return { x: base.x + offsets[node.skillId].x, y: base.y + offsets[node.skillId].y };
    });
    for (let left = 0; left < positions.length; left += 1) for (let right = left + 1; right < positions.length; right += 1) {
      expect(Math.hypot(positions[left].x - positions[right].x, positions[left].y - positions[right].y)).toBeGreaterThanOrEqual(61.7);
    }
  });

  it("moves a whole cluster and pushes an overlapping cluster away", () => {
    const twoClusters: SkillGraphSnapshot = {
      ...graph,
      graphVersion: "two-clusters",
      layoutVersion: "two-clusters",
      clusters: [
        graph.clusters[0],
        { clusterId: "cluster-b", name: "文档", summary: "文档能力", memberSkillIds: ["d", "e"], coreSkillIds: ["d", "e"], peripheral: false },
      ],
      nodes: [
        ...graph.nodes,
        { skillId: "d", name: "D", path: "C:/d", enabledAgents: ["cursor"], clusterId: "cluster-b", centrality: 0.9, superseded: false },
        { skillId: "e", name: "E", path: "C:/e", enabledAgents: ["cursor"], clusterId: "cluster-b", centrality: 0.86, superseded: false },
      ],
    };
    const layout = calculateSkillGraphLayout(twoClusters);
    const first = layout.clusterPoints.get("cluster-a")!;
    const second = layout.clusterPoints.get("cluster-b")!;
    const offsets = propagateClusterDragOffsets(
      twoClusters,
      layout,
      "cluster-a",
      { x: second.x - first.x, y: second.y - first.y },
      {},
    );
    expect(Math.hypot(offsets["cluster-a"].x, offsets["cluster-a"].y)).toBeGreaterThan(1);
    expect(Math.hypot(offsets["cluster-b"].x, offsets["cluster-b"].y)).toBeGreaterThan(1);
    const movedFirst = { x: first.x + offsets["cluster-a"].x, y: first.y + offsets["cluster-a"].y };
    const movedSecond = { x: second.x + offsets["cluster-b"].x, y: second.y + offsets["cluster-b"].y };
    const minimum = layout.clusterRadii.get("cluster-a")! + layout.clusterRadii.get("cluster-b")! + 29;
    expect(Math.hypot(movedFirst.x - movedSecond.x, movedFirst.y - movedSecond.y)).toBeGreaterThanOrEqual(minimum);
  });
});
