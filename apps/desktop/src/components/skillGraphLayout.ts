import type { SkillGraphEdge, SkillGraphNode } from "../types";

export interface GraphPoint {
  x: number;
  y: number;
}

const WIDTH = 1000;
const HEIGHT = 700;
const CENTER = { x: WIDTH / 2, y: HEIGHT / 2 };
const NODE_GAP = 58;
const CENTER_CLEARANCE = 82;
const LAYOUT_PADDING = 38;
const layoutCache = new Map<string, Map<string, GraphPoint>>();

export const graphViewport = {
  width: WIDTH,
  height: HEIGHT,
  center: CENTER,
};

export function getSkillGraphLayout(
  cacheKey: string,
  nodes: SkillGraphNode[],
  edges: SkillGraphEdge[],
): Map<string, GraphPoint> {
  const cached = layoutCache.get(cacheKey);
  if (cached && nodes.every((node) => cached.has(node.skillId))) {
    return cached;
  }
  const layout = calculateSkillGraphLayout(nodes, edges);
  layoutCache.set(cacheKey, layout);
  return layout;
}

export function calculateSkillGraphLayout(
  nodes: SkillGraphNode[],
  edges: SkillGraphEdge[],
): Map<string, GraphPoint> {
  const ordered = [...nodes].sort((left, right) =>
    left.skillId.localeCompare(right.skillId),
  );
  const points = new Map<string, GraphPoint>();
  const velocities = new Map<string, GraphPoint>();
  ordered.forEach((node, index) => {
    const angle = index * 2.399963229728653 + seededUnit(node.skillId) * 0.4;
    const radius = 95 + 15 * Math.sqrt(index + 1);
    points.set(node.skillId, {
      x: CENTER.x + Math.cos(angle) * radius,
      y: CENTER.y + Math.sin(angle) * radius,
    });
    velocities.set(node.skillId, { x: 0, y: 0 });
  });

  const indexedEdges = edges
    .map((edge) => ({
      edge,
      source: points.get(edge.sourceSkillId),
      target: points.get(edge.targetSkillId),
    }))
    .filter(
      (
        item,
      ): item is {
        edge: SkillGraphEdge;
        source: GraphPoint;
        target: GraphPoint;
      } => Boolean(item.source && item.target),
    );

  for (let iteration = 0; iteration < 220; iteration += 1) {
    const cooling = 1 - iteration / 190;
    const forces = new Map(
      ordered.map((node) => [node.skillId, { x: 0, y: 0 }]),
    );

    for (const node of ordered) {
      const point = points.get(node.skillId)!;
      const force = forces.get(node.skillId)!;
      force.x += (CENTER.x - point.x) * 0.004;
      force.y += (CENTER.y - point.y) * 0.004;
    }

    for (const { edge, source, target } of indexedEdges) {
      const dx = target.x - source.x;
      const dy = target.y - source.y;
      const distance = Math.max(1, Math.hypot(dx, dy));
      const desired = Math.max(
        72,
        148 - Math.max(0, edge.similarity) * 56 - edge.relations.length * 5,
      );
      const strength = edge.nearestFallback ? 0.012 : 0.024;
      const pull = (distance - desired) * strength;
      const fx = (dx / distance) * pull;
      const fy = (dy / distance) * pull;
      forces.get(edge.sourceSkillId)!.x += fx;
      forces.get(edge.sourceSkillId)!.y += fy;
      forces.get(edge.targetSkillId)!.x -= fx;
      forces.get(edge.targetSkillId)!.y -= fy;
    }

    applyLocalRepulsion(ordered, points, forces);
    applyCenterRepulsion(ordered, points, forces);

    for (const node of ordered) {
      const point = points.get(node.skillId)!;
      const velocity = velocities.get(node.skillId)!;
      const force = forces.get(node.skillId)!;
      velocity.x = (velocity.x + force.x * cooling) * 0.76;
      velocity.y = (velocity.y + force.y * cooling) * 0.76;
      point.x = clamp(point.x + velocity.x, LAYOUT_PADDING, WIDTH - LAYOUT_PADDING);
      point.y = clamp(point.y + velocity.y, LAYOUT_PADDING, HEIGHT - LAYOUT_PADDING);
    }
  }
  resolveHardCollisions(ordered, points);
  return points;
}

function applyCenterRepulsion(
  nodes: SkillGraphNode[],
  points: Map<string, GraphPoint>,
  forces: Map<string, GraphPoint>,
) {
  for (const node of nodes) {
    const point = points.get(node.skillId)!;
    let dx = point.x - CENTER.x;
    let dy = point.y - CENTER.y;
    let distance = Math.hypot(dx, dy);
    if (distance < 0.01) {
      const angle = seededUnit(`center:${node.skillId}`) * Math.PI * 2;
      dx = Math.cos(angle);
      dy = Math.sin(angle);
      distance = 1;
    }
    if (distance >= CENTER_CLEARANCE) continue;
    const push = (CENTER_CLEARANCE - distance) * 0.18;
    forces.get(node.skillId)!.x += (dx / distance) * push;
    forces.get(node.skillId)!.y += (dy / distance) * push;
  }
}

function resolveHardCollisions(
  nodes: SkillGraphNode[],
  points: Map<string, GraphPoint>,
) {
  for (let iteration = 0; iteration < 180; iteration += 1) {
    let changed = false;
    for (const node of nodes) {
      const point = points.get(node.skillId)!;
      let dx = point.x - CENTER.x;
      let dy = point.y - CENTER.y;
      let distance = Math.hypot(dx, dy);
      if (distance < CENTER_CLEARANCE) {
        if (distance < 0.01) {
          const angle = seededUnit(`final-center:${node.skillId}`) * Math.PI * 2;
          dx = Math.cos(angle);
          dy = Math.sin(angle);
          distance = 1;
        }
        point.x = CENTER.x + (dx / distance) * CENTER_CLEARANCE;
        point.y = CENTER.y + (dy / distance) * CENTER_CLEARANCE;
        changed = true;
      }
    }
    for (let leftIndex = 0; leftIndex < nodes.length; leftIndex += 1) {
      const left = points.get(nodes[leftIndex].skillId)!;
      for (let rightIndex = leftIndex + 1; rightIndex < nodes.length; rightIndex += 1) {
        const right = points.get(nodes[rightIndex].skillId)!;
        let dx = right.x - left.x;
        let dy = right.y - left.y;
        let distance = Math.hypot(dx, dy);
        if (distance >= NODE_GAP) continue;
        if (distance < 0.01) {
          const angle = seededUnit(`${nodes[leftIndex].skillId}:${nodes[rightIndex].skillId}`) * Math.PI * 2;
          dx = Math.cos(angle);
          dy = Math.sin(angle);
          distance = 1;
        }
        const shift = (NODE_GAP - distance) / 2 + 0.05;
        const x = (dx / distance) * shift;
        const y = (dy / distance) * shift;
        left.x = clamp(left.x - x, LAYOUT_PADDING, WIDTH - LAYOUT_PADDING);
        left.y = clamp(left.y - y, LAYOUT_PADDING, HEIGHT - LAYOUT_PADDING);
        right.x = clamp(right.x + x, LAYOUT_PADDING, WIDTH - LAYOUT_PADDING);
        right.y = clamp(right.y + y, LAYOUT_PADDING, HEIGHT - LAYOUT_PADDING);
        changed = true;
      }
    }
    if (!changed) break;
  }
}

function applyLocalRepulsion(
  nodes: SkillGraphNode[],
  points: Map<string, GraphPoint>,
  forces: Map<string, GraphPoint>,
) {
  const cellSize = NODE_GAP * 1.4;
  const cells = new Map<string, SkillGraphNode[]>();
  for (const node of nodes) {
    const point = points.get(node.skillId)!;
    const key = `${Math.floor(point.x / cellSize)}:${Math.floor(point.y / cellSize)}`;
    const cell = cells.get(key) ?? [];
    cell.push(node);
    cells.set(key, cell);
  }
  for (const node of nodes) {
    const point = points.get(node.skillId)!;
    const cellX = Math.floor(point.x / cellSize);
    const cellY = Math.floor(point.y / cellSize);
    for (let offsetX = -1; offsetX <= 1; offsetX += 1) {
      for (let offsetY = -1; offsetY <= 1; offsetY += 1) {
        const neighbors = cells.get(`${cellX + offsetX}:${cellY + offsetY}`) ?? [];
        for (const neighbor of neighbors) {
          if (neighbor.skillId <= node.skillId) continue;
          const other = points.get(neighbor.skillId)!;
          let dx = other.x - point.x;
          let dy = other.y - point.y;
          let distance = Math.hypot(dx, dy);
          if (distance < 0.01) {
            dx = seededUnit(`${node.skillId}:${neighbor.skillId}`) - 0.5;
            dy = seededUnit(`${neighbor.skillId}:${node.skillId}`) - 0.5;
            distance = Math.max(0.01, Math.hypot(dx, dy));
          }
          if (distance >= NODE_GAP) continue;
          const push = ((NODE_GAP - distance) / NODE_GAP) * 2.8;
          const fx = (dx / distance) * push;
          const fy = (dy / distance) * push;
          forces.get(node.skillId)!.x -= fx;
          forces.get(node.skillId)!.y -= fy;
          forces.get(neighbor.skillId)!.x += fx;
          forces.get(neighbor.skillId)!.y += fy;
        }
      }
    }
  }
}

export function parallelEdgeOffset(index: number, count: number): number {
  return (index - (count - 1) / 2) * 7;
}

function seededUnit(value: string): number {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0) / 4294967295;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
}
