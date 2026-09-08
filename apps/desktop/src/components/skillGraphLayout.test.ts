import { describe, expect, it } from "vitest";
import type { SkillGraphSnapshot } from "../types";
import { calculateDynamicClusterGeometry, calculateSkillGraphLayout, parallelEdgeOffset, propagateInteractiveDrag } from "./skillGraphLayout";

const graph: SkillGraphSnapshot = {
  graphVersion: "graph",
  layoutVersion: "layout",
  profileId: "profile",
  viewId: "cursor",
  excludedUnreadyCount: 0,
  excludedUnconnectedCount: 0,
  clusters: [{ clusterId: "cluster-a", name: "开发", summary: "开发能力", memberSkillIds: ["a", "b"], coreSkillIds: ["a", "b"], peripheral: false, semanticStatus: "ready" }],
  nodes: [
    { skillId: "a", name: "A", path: "C:/a", enabledAgents: ["cursor"], clusterId: "cluster-a", centrality: 0.94, superseded: false, disabled: false, classificationStatus: "ready" },
    { skillId: "b", name: "B", path: "C:/b", enabledAgents: ["cursor"], clusterId: "cluster-a", centrality: 0.9, superseded: false, disabled: false, classificationStatus: "ready" },
    { skillId: "c", name: "C", path: "C:/c", enabledAgents: ["cursor"], centrality: 0, superseded: false, disabled: false, classificationStatus: "ready" },
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
    const result = propagateInteractiveDrag(graph, layout, { type: "skill", id: "a", offset: { x: 140, y: 20 } }, {}, {});
    expect(Math.hypot(result.nodeOffsets.a.x, result.nodeOffsets.a.y)).toBeGreaterThan(100);
    expect(Math.hypot(result.nodeOffsets.b.x, result.nodeOffsets.b.y)).toBeGreaterThan(1);
    expect(Math.hypot(result.centerOffsets["cluster-a"].x, result.centerOffsets["cluster-a"].y)).toBeGreaterThan(1);

    const positions = graph.nodes.map((node) => {
      const base = layout.nodePoints.get(node.skillId)!;
      return { x: base.x + result.nodeOffsets[node.skillId].x, y: base.y + result.nodeOffsets[node.skillId].y };
    });
    for (let left = 0; left < positions.length; left += 1) for (let right = left + 1; right < positions.length; right += 1) {
      expect(Math.hypot(positions[left].x - positions[right].x, positions[left].y - positions[right].y)).toBeGreaterThanOrEqual(61.7);
    }
  });

  it("drags a cluster center with the same spring physics without rigidly moving members", () => {
    const layout = calculateSkillGraphLayout(graph);
    const result = propagateInteractiveDrag(graph, layout, { type: "cluster", id: "cluster-a", offset: { x: 120, y: 30 } }, {}, {});
    expect(Math.hypot(result.centerOffsets["cluster-a"].x, result.centerOffsets["cluster-a"].y)).toBeGreaterThan(100);
    expect(Math.hypot(result.nodeOffsets.a.x, result.nodeOffsets.a.y)).toBeGreaterThan(1);
    expect(result.nodeOffsets.a).not.toEqual(result.centerOffsets["cluster-a"]);
    const centerBase = layout.clusterPoints.get("cluster-a")!;
    const center = { x: centerBase.x + result.centerOffsets["cluster-a"].x, y: centerBase.y + result.centerOffsets["cluster-a"].y };
    const geometry = calculateDynamicClusterGeometry(
      graph.clusters[0].memberSkillIds.map((id) => {
        const base = layout.nodePoints.get(id)!;
        return { x: base.x + result.nodeOffsets[id].x, y: base.y + result.nodeOffsets[id].y };
      }),
      center,
    );
    expect(Math.hypot(center.x - geometry.x, center.y - geometry.y)).toBeGreaterThan(1);
    expect(graph.clusters[0].memberSkillIds.every((id) => {
      const base = layout.nodePoints.get(id)!;
      const point = { x: base.x + result.nodeOffsets[id].x, y: base.y + result.nodeOffsets[id].y };
      return Math.hypot(point.x - geometry.x, point.y - geometry.y) + 19 <= geometry.radius;
    })).toBe(true);
  });

  it("keeps the full peripheral node outside the dynamic cluster boundary", () => {
    const layout = calculateSkillGraphLayout(graph);
    const cluster = graph.clusters[0];
    const geometry = calculateDynamicClusterGeometry(
      cluster.memberSkillIds.map((id) => layout.nodePoints.get(id)!),
      layout.clusterPoints.get(cluster.clusterId)!,
    );
    const peripheral = layout.nodePoints.get("c")!;
    expect(Math.hypot(peripheral.x - geometry.x, peripheral.y - geometry.y)).toBeGreaterThanOrEqual(geometry.radius + 24.8);
  });
});
