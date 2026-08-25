import type { SkillGraphCluster, SkillGraphSnapshot } from "../types";

export interface GraphPoint { x: number; y: number }
export interface SkillGraphLayout {
  nodePoints: Map<string, GraphPoint>;
  clusterPoints: Map<string, GraphPoint>;
  clusterRadii: Map<string, number>;
}

const WIDTH = 1200;
const HEIGHT = 800;
const CENTER = { x: WIDTH / 2, y: HEIGHT / 2 };
const PADDING = 54;
const NODE_GAP = 62;
const layoutCache = new Map<string, SkillGraphLayout>();
export const graphViewport = { width: WIDTH, height: HEIGHT, center: CENTER };

export function getSkillGraphLayout(cacheKey: string, graph: SkillGraphSnapshot): SkillGraphLayout {
  const cached = layoutCache.get(cacheKey);
  if (cached && graph.nodes.every((node) => cached.nodePoints.has(node.skillId)) && graph.clusters.every((cluster) => cached.clusterPoints.has(cluster.clusterId))) return cached;
  const layout = calculateSkillGraphLayout(graph);
  layoutCache.set(cacheKey, layout);
  return layout;
}

export function calculateSkillGraphLayout(graph: SkillGraphSnapshot): SkillGraphLayout {
  const clusterRadii = new Map(graph.clusters.map((cluster) => [cluster.clusterId, Math.max(96, 56 + Math.sqrt(cluster.memberSkillIds.length) * 48)]));
  const clusterPoints = placeClusters(graph.clusters, clusterRadii, graph);
  const nodePoints = new Map<string, GraphPoint>();
  const nodesById = new Map(graph.nodes.map((node) => [node.skillId, node]));
  for (const cluster of graph.clusters) {
    const center = clusterPoints.get(cluster.clusterId) ?? CENTER;
    const radius = clusterRadii.get(cluster.clusterId) ?? 110;
    const members = cluster.memberSkillIds.map((id) => nodesById.get(id)).filter((node): node is NonNullable<typeof node> => Boolean(node)).sort((left, right) => right.centrality - left.centrality || left.skillId.localeCompare(right.skillId));
    members.forEach((node, index) => {
      const ring = cluster.coreSkillIds.includes(node.skillId) ? 50 + index * 8 : 70 + (1 - clamp(node.centrality, 0, 1)) * (radius - 76);
      const angle = index * 2.399963229728653 + seededUnit(node.skillId) * 0.5;
      nodePoints.set(node.skillId, { x: center.x + Math.cos(angle) * ring, y: center.y + Math.sin(angle) * ring });
    });
  }
  const peripheral = graph.nodes.filter((node) => !node.clusterId).sort((left, right) => left.skillId.localeCompare(right.skillId));
  peripheral.forEach((node, index) => {
    const anchor = closestCluster(node.skillId, graph, clusterPoints);
    const fallback = index / Math.max(1, peripheral.length) * Math.PI * 2 + seededUnit(node.skillId) * 0.2;
    const angle = anchor ? Math.atan2(anchor.y - CENTER.y, anchor.x - CENTER.x) : fallback;
    const spread = (index % 3 - 1) * 0.12 + (seededUnit(`cloud:${node.skillId}`) - 0.5) * 0.12;
    const radius = Math.min(WIDTH, HEIGHT) * 0.43;
    nodePoints.set(node.skillId, { x: CENTER.x + Math.cos(angle + spread) * radius, y: CENTER.y + Math.sin(angle + spread) * radius });
  });
  refineClusterNodes(graph, nodePoints, clusterPoints, clusterRadii);
  resolveNodeCollisions(nodePoints);
  return { nodePoints, clusterPoints, clusterRadii };
}

export function propagateDragOffsets(
  graph: SkillGraphSnapshot,
  layout: SkillGraphLayout,
  draggedSkillId: string,
  draggedOffset: GraphPoint,
  previousOffsets: Record<string, GraphPoint>,
): Record<string, GraphPoint> {
  const positions = new Map(
    graph.nodes.map((node) => {
      const base = layout.nodePoints.get(node.skillId) ?? CENTER;
      const previous = previousOffsets[node.skillId] ?? { x: 0, y: 0 };
      return [node.skillId, { x: base.x + previous.x, y: base.y + previous.y }];
    }),
  );
  const draggedBase = layout.nodePoints.get(draggedSkillId) ?? CENTER;
  const draggedTarget = {
    x: clamp(draggedBase.x + draggedOffset.x, PADDING, WIDTH - PADDING),
    y: clamp(draggedBase.y + draggedOffset.y, PADDING, HEIGHT - PADDING),
  };
  positions.set(draggedSkillId, draggedTarget);

  const springs = [
    ...graph.edges.map((edge) => ({
      source: edge.sourceSkillId,
      target: edge.targetSkillId,
      strength: 0.22,
    })),
    ...graph.proximities.map((relation) => ({
      source: relation.sourceSkillId,
      target: relation.targetSkillId,
      strength: 0.035 + relation.weight * 0.055,
    })),
  ].map((spring) => {
    const source = layout.nodePoints.get(spring.source);
    const target = layout.nodePoints.get(spring.target);
    return {
      ...spring,
      rest: source && target ? Math.max(NODE_GAP, Math.hypot(target.x - source.x, target.y - source.y)) : NODE_GAP,
    };
  });

  for (let iteration = 0; iteration < 14; iteration += 1) {
    const forces = new Map(graph.nodes.map((node) => [node.skillId, { x: 0, y: 0 }]));
    for (const spring of springs) {
      const source = positions.get(spring.source);
      const target = positions.get(spring.target);
      if (!source || !target) continue;
      const delta = safeDelta(source, target, `drag:${spring.source}:${spring.target}`);
      const pull = (delta.distance - spring.rest) * spring.strength;
      if (spring.source !== draggedSkillId) {
        forces.get(spring.source)!.x += delta.x * pull;
        forces.get(spring.source)!.y += delta.y * pull;
      }
      if (spring.target !== draggedSkillId) {
        forces.get(spring.target)!.x -= delta.x * pull;
        forces.get(spring.target)!.y -= delta.y * pull;
      }
    }
    for (const node of graph.nodes) {
      if (node.skillId === draggedSkillId) continue;
      const point = positions.get(node.skillId)!;
      const base = layout.nodePoints.get(node.skillId) ?? CENTER;
      const force = forces.get(node.skillId)!;
      force.x += (base.x - point.x) * 0.035;
      force.y += (base.y - point.y) * 0.035;
      point.x = clamp(point.x + force.x * 0.58, PADDING, WIDTH - PADDING);
      point.y = clamp(point.y + force.y * 0.58, PADDING, HEIGHT - PADDING);
    }
    resolveInteractiveCollisions(positions, draggedSkillId);
    positions.set(draggedSkillId, draggedTarget);
  }

  return Object.fromEntries(
    graph.nodes.map((node) => {
      const base = layout.nodePoints.get(node.skillId) ?? CENTER;
      const point = positions.get(node.skillId) ?? base;
      return [node.skillId, { x: point.x - base.x, y: point.y - base.y }];
    }),
  );
}

export function propagateClusterDragOffsets(
  graph: SkillGraphSnapshot,
  layout: SkillGraphLayout,
  draggedClusterId: string,
  draggedOffset: GraphPoint,
  previousOffsets: Record<string, GraphPoint>,
): Record<string, GraphPoint> {
  const positions = new Map(
    graph.clusters.map((cluster) => {
      const base = layout.clusterPoints.get(cluster.clusterId) ?? CENTER;
      const previous = previousOffsets[cluster.clusterId] ?? { x: 0, y: 0 };
      return [cluster.clusterId, { x: base.x + previous.x, y: base.y + previous.y }];
    }),
  );
  const draggedBase = layout.clusterPoints.get(draggedClusterId) ?? CENTER;
  const draggedRadius = layout.clusterRadii.get(draggedClusterId) ?? 100;
  const draggedTarget = {
    x: clamp(draggedBase.x + draggedOffset.x, PADDING + draggedRadius, WIDTH - PADDING - draggedRadius),
    y: clamp(draggedBase.y + draggedOffset.y, PADDING + draggedRadius, HEIGHT - PADDING - draggedRadius),
  };
  positions.set(draggedClusterId, draggedTarget);

  for (let iteration = 0; iteration < 16; iteration += 1) {
    for (const cluster of graph.clusters) {
      if (cluster.clusterId === draggedClusterId) continue;
      const point = positions.get(cluster.clusterId)!;
      const base = layout.clusterPoints.get(cluster.clusterId) ?? CENTER;
      point.x += (base.x - point.x) * 0.035;
      point.y += (base.y - point.y) * 0.035;
    }
    resolveClusterCollisions(graph.clusters, positions, layout.clusterRadii, draggedClusterId);
    positions.set(draggedClusterId, draggedTarget);
  }

  return Object.fromEntries(
    graph.clusters.map((cluster) => {
      const base = layout.clusterPoints.get(cluster.clusterId) ?? CENTER;
      const point = positions.get(cluster.clusterId) ?? base;
      return [cluster.clusterId, { x: point.x - base.x, y: point.y - base.y }];
    }),
  );
}

function resolveClusterCollisions(
  clusters: SkillGraphCluster[],
  points: Map<string, GraphPoint>,
  radii: Map<string, number>,
  fixedId: string,
) {
  for (let pass = 0; pass < 6; pass += 1) {
    let changed = false;
    for (let leftIndex = 0; leftIndex < clusters.length; leftIndex += 1) for (let rightIndex = leftIndex + 1; rightIndex < clusters.length; rightIndex += 1) {
      const leftId = clusters[leftIndex].clusterId;
      const rightId = clusters[rightIndex].clusterId;
      const left = points.get(leftId)!;
      const right = points.get(rightId)!;
      const delta = safeDelta(left, right, `cluster-drag:${leftId}:${rightId}`);
      const minimum = (radii.get(leftId) ?? 100) + (radii.get(rightId) ?? 100) + 30;
      if (delta.distance >= minimum) continue;
      const overlap = minimum - delta.distance + 0.25;
      if (leftId === fixedId) {
        moveClusterPoint(right, delta.x * overlap, delta.y * overlap, radii.get(rightId) ?? 100);
      } else if (rightId === fixedId) {
        moveClusterPoint(left, -delta.x * overlap, -delta.y * overlap, radii.get(leftId) ?? 100);
      } else {
        moveClusterPoint(left, -delta.x * overlap * 0.5, -delta.y * overlap * 0.5, radii.get(leftId) ?? 100);
        moveClusterPoint(right, delta.x * overlap * 0.5, delta.y * overlap * 0.5, radii.get(rightId) ?? 100);
      }
      changed = true;
    }
    if (!changed) break;
  }
}

function moveClusterPoint(point: GraphPoint, dx: number, dy: number, radius: number) {
  point.x = clamp(point.x + dx, PADDING + radius, WIDTH - PADDING - radius);
  point.y = clamp(point.y + dy, PADDING + radius, HEIGHT - PADDING - radius);
}

function resolveInteractiveCollisions(points: Map<string, GraphPoint>, fixedId: string) {
  const entries = [...points.entries()];
  for (let pass = 0; pass < 4; pass += 1) {
    let changed = false;
    for (let leftIndex = 0; leftIndex < entries.length; leftIndex += 1) for (let rightIndex = leftIndex + 1; rightIndex < entries.length; rightIndex += 1) {
      const [leftId, left] = entries[leftIndex];
      const [rightId, right] = entries[rightIndex];
      const delta = safeDelta(left, right, `interactive:${leftId}:${rightId}`);
      if (delta.distance >= NODE_GAP) continue;
      const overlap = NODE_GAP - delta.distance + 0.15;
      if (leftId === fixedId) {
        right.x = clamp(right.x + delta.x * overlap, PADDING, WIDTH - PADDING);
        right.y = clamp(right.y + delta.y * overlap, PADDING, HEIGHT - PADDING);
      } else if (rightId === fixedId) {
        left.x = clamp(left.x - delta.x * overlap, PADDING, WIDTH - PADDING);
        left.y = clamp(left.y - delta.y * overlap, PADDING, HEIGHT - PADDING);
      } else {
        left.x = clamp(left.x - delta.x * overlap * 0.5, PADDING, WIDTH - PADDING);
        left.y = clamp(left.y - delta.y * overlap * 0.5, PADDING, HEIGHT - PADDING);
        right.x = clamp(right.x + delta.x * overlap * 0.5, PADDING, WIDTH - PADDING);
        right.y = clamp(right.y + delta.y * overlap * 0.5, PADDING, HEIGHT - PADDING);
      }
      changed = true;
    }
    if (!changed) break;
  }
}

function refineClusterNodes(graph: SkillGraphSnapshot, points: Map<string, GraphPoint>, centers: Map<string, GraphPoint>, radii: Map<string, number>) {
  const nodes = new Map(graph.nodes.map((node) => [node.skillId, node]));
  const anchors = new Map([...points].map(([id, point]) => [id, { ...point }]));
  const velocities = new Map(graph.nodes.map((node) => [node.skillId, { x: 0, y: 0 }]));
  const springs: Array<{ source: string; target: string; strength: number; desired: number }> = [];
  graph.edges.forEach((edge) => {
    const source = nodes.get(edge.sourceSkillId);
    const target = nodes.get(edge.targetSkillId);
    if (source?.clusterId && source.clusterId === target?.clusterId) springs.push({ source: edge.sourceSkillId, target: edge.targetSkillId, strength: 0.02, desired: 76 + (1 - edge.similarity) * 34 });
  });
  graph.proximities.forEach((relation) => {
    const source = nodes.get(relation.sourceSkillId);
    const target = nodes.get(relation.targetSkillId);
    if (source?.clusterId && source.clusterId === target?.clusterId) springs.push({ source: relation.sourceSkillId, target: relation.targetSkillId, strength: 0.004 + relation.weight * 0.005, desired: 104 });
  });
  for (let iteration = 0; iteration < 100; iteration += 1) {
    const forces = new Map(graph.nodes.map((node) => [node.skillId, { x: 0, y: 0 }]));
    for (const spring of springs) {
      const source = points.get(spring.source)!;
      const target = points.get(spring.target)!;
      const delta = safeDelta(source, target, `${spring.source}:${spring.target}`);
      const pull = (delta.distance - spring.desired) * spring.strength;
      forces.get(spring.source)!.x += delta.x * pull;
      forces.get(spring.source)!.y += delta.y * pull;
      forces.get(spring.target)!.x -= delta.x * pull;
      forces.get(spring.target)!.y -= delta.y * pull;
    }
    for (const node of graph.nodes.filter((item) => item.clusterId)) {
      const point = points.get(node.skillId)!;
      const anchor = anchors.get(node.skillId)!;
      const force = forces.get(node.skillId)!;
      force.x += (anchor.x - point.x) * 0.025;
      force.y += (anchor.y - point.y) * 0.025;
      const speed = velocities.get(node.skillId)!;
      speed.x = (speed.x + force.x) * 0.7;
      speed.y = (speed.y + force.y) * 0.7;
      point.x += speed.x;
      point.y += speed.y;
      const center = centers.get(node.clusterId!)!;
      const radius = (radii.get(node.clusterId!) ?? 100) - 24;
      const delta = safeDelta(center, point, `bound:${node.skillId}`);
      if (delta.distance > radius) {
        point.x = center.x + delta.x * radius;
        point.y = center.y + delta.y * radius;
      }
    }
  }
}

function placeClusters(clusters: SkillGraphCluster[], radii: Map<string, number>, graph: SkillGraphSnapshot): Map<string, GraphPoint> {
  const points = new Map<string, GraphPoint>();
  const velocity = new Map<string, GraphPoint>();
  clusters.forEach((cluster, index) => {
    const angle = index * 2.399963229728653 + seededUnit(cluster.clusterId);
    const radius = 90 + Math.sqrt(index + 1) * 75;
    points.set(cluster.clusterId, { x: CENTER.x + Math.cos(angle) * radius, y: CENTER.y + Math.sin(angle) * radius * 0.72 });
    velocity.set(cluster.clusterId, { x: 0, y: 0 });
  });
  const clusterBySkill = new Map(graph.nodes.filter((node) => node.clusterId).map((node) => [node.skillId, node.clusterId!]));
  const attractions = new Map<string, number>();
  const add = (sourceId: string, targetId: string, weight: number) => {
    const source = clusterBySkill.get(sourceId);
    const target = clusterBySkill.get(targetId);
    if (!source || !target || source === target) return;
    const key = orderedPair(source, target).join("\0");
    attractions.set(key, (attractions.get(key) ?? 0) + weight);
  };
  graph.edges.forEach((edge) => add(edge.sourceSkillId, edge.targetSkillId, 0.8 + edge.similarity));
  graph.proximities.forEach((relation) => add(relation.sourceSkillId, relation.targetSkillId, relation.weight * 0.35));
  for (let iteration = 0; iteration < 240; iteration += 1) {
    const forces = new Map(clusters.map((cluster) => [cluster.clusterId, { x: 0, y: 0 }]));
    for (let leftIndex = 0; leftIndex < clusters.length; leftIndex += 1) {
      for (let rightIndex = leftIndex + 1; rightIndex < clusters.length; rightIndex += 1) {
        const leftId = clusters[leftIndex].clusterId;
        const rightId = clusters[rightIndex].clusterId;
        const left = points.get(leftId)!;
        const right = points.get(rightId)!;
        const delta = safeDelta(left, right, `${leftId}:${rightId}`);
        const minimum = (radii.get(leftId) ?? 100) + (radii.get(rightId) ?? 100) + 54;
        const repulsion = Math.max(0, minimum - delta.distance) * 0.045;
        const relation = attractions.get(orderedPair(leftId, rightId).join("\0")) ?? 0;
        const attraction = relation > 0 ? (delta.distance - minimum * 1.08) * Math.min(0.012, relation * 0.0025) : 0;
        const force = attraction - repulsion;
        forces.get(leftId)!.x += delta.x * force;
        forces.get(leftId)!.y += delta.y * force;
        forces.get(rightId)!.x -= delta.x * force;
        forces.get(rightId)!.y -= delta.y * force;
      }
    }
    for (const cluster of clusters) {
      const point = points.get(cluster.clusterId)!;
      const force = forces.get(cluster.clusterId)!;
      force.x += (CENTER.x - point.x) * 0.0015;
      force.y += (CENTER.y - point.y) * 0.0015;
      const speed = velocity.get(cluster.clusterId)!;
      speed.x = (speed.x + force.x) * 0.72;
      speed.y = (speed.y + force.y) * 0.72;
      const radius = radii.get(cluster.clusterId) ?? 100;
      point.x = clamp(point.x + speed.x, PADDING + radius, WIDTH - PADDING - radius);
      point.y = clamp(point.y + speed.y, PADDING + radius, HEIGHT - PADDING - radius);
    }
  }
  return points;
}

function closestCluster(skillId: string, graph: SkillGraphSnapshot, points: Map<string, GraphPoint>): GraphPoint | undefined {
  const nodeCluster = new Map(graph.nodes.filter((node) => node.clusterId).map((node) => [node.skillId, node.clusterId!]));
  const candidates: Array<{ point: GraphPoint; score: number }> = [];
  const collect = (source: string, target: string, score: number) => {
    const other = source === skillId ? target : target === skillId ? source : undefined;
    const clusterId = other ? nodeCluster.get(other) : undefined;
    const point = clusterId ? points.get(clusterId) : undefined;
    if (point) candidates.push({ point, score });
  };
  graph.edges.forEach((edge) => collect(edge.sourceSkillId, edge.targetSkillId, edge.similarity + 1));
  graph.proximities.forEach((relation) => collect(relation.sourceSkillId, relation.targetSkillId, relation.weight));
  return candidates.sort((left, right) => right.score - left.score)[0]?.point;
}

function resolveNodeCollisions(points: Map<string, GraphPoint>) {
  const entries = [...points.entries()];
  for (let iteration = 0; iteration < 160; iteration += 1) {
    let changed = false;
    for (let leftIndex = 0; leftIndex < entries.length; leftIndex += 1) for (let rightIndex = leftIndex + 1; rightIndex < entries.length; rightIndex += 1) {
      const [leftId, left] = entries[leftIndex];
      const [rightId, right] = entries[rightIndex];
      const delta = safeDelta(left, right, `${leftId}:${rightId}`);
      if (delta.distance >= NODE_GAP) continue;
      const shift = (NODE_GAP - delta.distance) / 2 + 0.1;
      left.x = clamp(left.x - delta.x * shift, PADDING, WIDTH - PADDING);
      left.y = clamp(left.y - delta.y * shift, PADDING, HEIGHT - PADDING);
      right.x = clamp(right.x + delta.x * shift, PADDING, WIDTH - PADDING);
      right.y = clamp(right.y + delta.y * shift, PADDING, HEIGHT - PADDING);
      changed = true;
    }
    if (!changed) break;
  }
}

function safeDelta(left: GraphPoint, right: GraphPoint, seed: string) {
  let dx = right.x - left.x;
  let dy = right.y - left.y;
  let distance = Math.hypot(dx, dy);
  if (distance < 0.01) {
    const angle = seededUnit(seed) * Math.PI * 2;
    dx = Math.cos(angle); dy = Math.sin(angle); distance = 1;
  }
  return { x: dx / distance, y: dy / distance, distance };
}

export function parallelEdgeOffset(index: number, count: number): number { return (index - (count - 1) / 2) * 7; }
function seededUnit(value: string): number { let hash = 2166136261; for (let index = 0; index < value.length; index += 1) { hash ^= value.charCodeAt(index); hash = Math.imul(hash, 16777619); } return (hash >>> 0) / 4294967295; }
function orderedPair(left: string, right: string): [string, string] { return left <= right ? [left, right] : [right, left]; }
function clamp(value: number, minimum: number, maximum: number): number { return Math.min(maximum, Math.max(minimum, value)); }
