import {
  type PointerEvent as ReactPointerEvent,
  type WheelEvent,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { api } from "../api";
import type {
  SkillGraphEdge,
  SkillGraphCluster,
  SkillGraphNode,
  SkillGraphSnapshot,
  SkillGraphRelation,
  ViewId,
} from "../types";
import {
  getSkillGraphLayout,
  graphViewport,
  parallelEdgeOffset,
  calculateDynamicClusterGeometry,
  propagateInteractiveDrag,
  type GraphPoint,
} from "./skillGraphLayout";

interface SkillGraphProps {
  activeView: ViewId;
  isNative: boolean;
  refreshKey: number;
}

type Selection =
  | { type: "node"; nodeId: string }
  | { type: "edge"; edgeId: string }
  | { type: "cluster"; clusterId: string }
  | null;

type ActiveDrag =
  | {
      type: "node";
      nodeId: string;
      startX: number;
      startY: number;
      origin: GraphPoint;
    }
  | {
      type: "canvas";
      startX: number;
      startY: number;
      origin: GraphPoint;
    }
  | {
      type: "cluster";
      clusterId: string;
      startX: number;
      startY: number;
      origin: GraphPoint;
    };

const viewLabels: Record<ViewId, string> = {
  all: "所有 Skills",
  "claude-code": "Claude Code",
  cursor: "Cursor",
  codex: "Codex",
};

const relationLabels: Record<string, string> = {
  similar_to: "功能相似",
  overlaps_with: "能力或流程重叠",
  duplicate_candidate: "重复候选",
  supersedes: "替代关系",
  depends_on: "前置或依赖",
  reads_reference: "共享参考资料",
  runs_script: "共享脚本",
  uses_asset: "共享资源",
  located_in: "相同来源位置",
};

const stateLabels: Record<string, string> = {
  carried_fact: "已带入事实",
  human_confirmed: "人工确认",
  migration_hint: "迁移提示",
  revalidated: "已复核",
  over_threshold: "超过可视化阈值",
  nearest_fallback: "最近无冲突节点",
};

export function SkillGraph({
  activeView,
  isNative,
  refreshKey,
}: SkillGraphProps) {
  const [graph, setGraph] = useState<SkillGraphSnapshot>();
  const [state, setState] = useState<"loading" | "ready" | "error">(
    isNative ? "loading" : "ready",
  );
  const [error, setError] = useState("");
  const [selection, setSelection] = useState<Selection>(null);
  const [zoom, setZoom] = useState(1);
  const [panOffset, setPanOffset] = useState<GraphPoint>({ x: 0, y: 0 });
  const [dragOffsets, setDragOffsets] = useState<Record<string, GraphPoint>>({});
  const [clusterOffsets, setClusterOffsets] = useState<Record<string, GraphPoint>>({});
  const svgRef = useRef<SVGSVGElement>(null);
  const activeDragRef = useRef<ActiveDrag | undefined>(undefined);
  const panOffsetRef = useRef(panOffset);
  const dragOffsetsRef = useRef(dragOffsets);
  const clusterOffsetsRef = useRef(clusterOffsets);
  const returnAnimationRef = useRef<number | undefined>(undefined);

  useEffect(() => {
    setZoom(1);
    setPanOffset({ x: 0, y: 0 });
    panOffsetRef.current = { x: 0, y: 0 };
    setDragOffsets({});
    dragOffsetsRef.current = {};
    setClusterOffsets({});
    clusterOffsetsRef.current = {};
    setSelection(null);
  }, [activeView]);

  useEffect(() => {
    if (!isNative) {
      setGraph(undefined);
      setState("ready");
      return;
    }
    let current = true;
    setState("loading");
    setError("");
    api
      .getSkillGraph(activeView)
      .then((nextGraph) => {
        if (!current) return;
        const normalizedGraph: SkillGraphSnapshot = {
          ...nextGraph,
          layoutVersion: nextGraph.layoutVersion ?? nextGraph.graphVersion,
          clusters: nextGraph.clusters ?? [],
          proximities: nextGraph.proximities ?? [],
          nodes: nextGraph.nodes.map((node) => ({
            ...node,
            centrality: node.centrality ?? 0,
            superseded: node.superseded ?? false,
          })),
        };
        setGraph((previous) =>
          previous?.graphVersion === normalizedGraph.graphVersion &&
          previous.viewId === normalizedGraph.viewId
            ? previous
            : normalizedGraph,
        );
        setState("ready");
      })
      .catch((nextError) => {
        if (!current) return;
        setError(String(nextError));
        setState("error");
      });
    return () => {
      current = false;
    };
  }, [activeView, isNative, refreshKey]);

  useEffect(
    () => () => {
      if (returnAnimationRef.current) {
        cancelAnimationFrame(returnAnimationRef.current);
      }
    },
    [],
  );

  const layout = useMemo(
    () =>
      graph
        ? getSkillGraphLayout(`${graph.viewId}:${graph.layoutVersion}`, graph)
        : {
            nodePoints: new Map<string, GraphPoint>(),
            clusterPoints: new Map<string, GraphPoint>(),
            clusterRadii: new Map<string, number>(),
          },
    [graph],
  );
  const nodesById = useMemo(
    () =>
      new Map((graph?.nodes ?? []).map((node) => [node.skillId, node])),
    [graph],
  );
  const selectedNode =
    selection?.type === "node" ? nodesById.get(selection.nodeId) : undefined;
  const selectedEdge =
    selection?.type === "edge"
      ? graph?.edges.find((edge) => edge.edgeId === selection.edgeId)
      : undefined;
  const selectedCluster =
    selection?.type === "cluster"
      ? graph?.clusters.find((cluster) => cluster.clusterId === selection.clusterId)
      : undefined;

  const displayPoint = (skillId: string): GraphPoint => {
    const point = layout.nodePoints.get(skillId) ?? graphViewport.center;
    const offset = dragOffsets[skillId] ?? { x: 0, y: 0 };
    return { x: point.x + offset.x, y: point.y + offset.y };
  };
  const clusterPoint = (clusterId: string) => {
    const point = layout.clusterPoints.get(clusterId) ?? graphViewport.center;
    const offset = clusterOffsets[clusterId] ?? { x: 0, y: 0 };
    return { x: point.x + offset.x, y: point.y + offset.y };
  };
  const edgeFocusClass = (edge: SkillGraphEdge) => {
    const sourceCluster = nodesById.get(edge.sourceSkillId)?.clusterId;
    const targetCluster = nodesById.get(edge.targetSkillId)?.clusterId;
    const classes = [sourceCluster !== targetCluster ? "is-cross-cluster" : ""];
    if (selectedNode) {
      classes.push(
        edge.sourceSkillId === selectedNode.skillId || edge.targetSkillId === selectedNode.skillId
          ? "is-focused"
          : "is-muted",
      );
    } else if (selectedCluster) {
      classes.push(
        sourceCluster === selectedCluster.clusterId && targetCluster === selectedCluster.clusterId
          ? "is-focused"
          : "is-muted",
      );
    }
    return classes.filter(Boolean).join(" ");
  };

  const closeTransientDetails = () => setSelection(null);

  const onWheel = (event: WheelEvent<SVGSVGElement>) => {
    event.preventDefault();
    setZoom((current) =>
      Math.min(2.5, Math.max(0.55, current * (event.deltaY > 0 ? 0.9 : 1.1))),
    );
  };

  const beginCanvasDrag = (event: ReactPointerEvent<SVGSVGElement>) => {
    closeTransientDetails();
    if (event.button !== 0 || event.target !== event.currentTarget) return;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    activeDragRef.current = {
      type: "canvas",
      startX: event.clientX,
      startY: event.clientY,
      origin: panOffsetRef.current,
    };
  };

  const beginNodeDrag = (
    event: ReactPointerEvent<SVGGElement>,
    nodeId: string,
  ) => {
    if (event.button !== 0) return;
    event.stopPropagation();
    closeTransientDetails();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    activeDragRef.current = {
      type: "node",
      nodeId,
      startX: event.clientX,
      startY: event.clientY,
      origin: dragOffsetsRef.current[nodeId] ?? { x: 0, y: 0 },
    };
  };

  const beginClusterDrag = (
    event: ReactPointerEvent<SVGGElement>,
    clusterId: string,
  ) => {
    if (event.button !== 0) return;
    event.stopPropagation();
    closeTransientDetails();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    activeDragRef.current = {
      type: "cluster",
      clusterId,
      startX: event.clientX,
      startY: event.clientY,
      origin: clusterOffsetsRef.current[clusterId] ?? { x: 0, y: 0 },
    };
  };

  const moveDrag = (event: ReactPointerEvent<SVGSVGElement>) => {
    const drag = activeDragRef.current;
    const svg = svgRef.current;
    if (!drag || !svg) return;
    const bounds = svg.getBoundingClientRect();
    const dx =
      ((event.clientX - drag.startX) * graphViewport.width) /
      Math.max(1, bounds.width) /
      zoom;
    const dy =
      ((event.clientY - drag.startY) * graphViewport.height) /
      Math.max(1, bounds.height) /
      zoom;
    if (drag.type === "canvas") {
      const next = { x: drag.origin.x + dx, y: drag.origin.y + dy };
      panOffsetRef.current = next;
      setPanOffset(next);
      return;
    }
    if (!graph) return;
    if (drag.type === "cluster") {
      const next = propagateInteractiveDrag(
        graph,
        layout,
        { type: "cluster", id: drag.clusterId, offset: { x: drag.origin.x + dx, y: drag.origin.y + dy } },
        dragOffsetsRef.current,
        clusterOffsetsRef.current,
      );
      dragOffsetsRef.current = next.nodeOffsets;
      clusterOffsetsRef.current = next.centerOffsets;
      setDragOffsets(next.nodeOffsets);
      setClusterOffsets(next.centerOffsets);
      return;
    }
    const next = propagateInteractiveDrag(
      graph,
      layout,
      { type: "skill", id: drag.nodeId, offset: { x: drag.origin.x + dx, y: drag.origin.y + dy } },
      dragOffsetsRef.current,
      clusterOffsetsRef.current,
    );
    dragOffsetsRef.current = next.nodeOffsets;
    clusterOffsetsRef.current = next.centerOffsets;
    setDragOffsets(next.nodeOffsets);
    setClusterOffsets(next.centerOffsets);
  };

  const endDrag = () => {
    if (!activeDragRef.current) return;
    activeDragRef.current = undefined;
    animateReturn();
  };

  const animateReturn = () => {
    if (returnAnimationRef.current) {
      cancelAnimationFrame(returnAnimationRef.current);
    }
    const step = () => {
      const nextPan = {
        x: panOffsetRef.current.x * 0.78,
        y: panOffsetRef.current.y * 0.78,
      };
      const nextOffsets: Record<string, GraphPoint> = {};
      const nextClusterOffsets: Record<string, GraphPoint> = {};
      let moving = Math.hypot(nextPan.x, nextPan.y) > 0.25;
      for (const [skillId, offset] of Object.entries(dragOffsetsRef.current)) {
        const next = { x: offset.x * 0.78, y: offset.y * 0.78 };
        if (Math.hypot(next.x, next.y) > 0.25) {
          nextOffsets[skillId] = next;
          moving = true;
        }
      }
      for (const [clusterId, offset] of Object.entries(clusterOffsetsRef.current)) {
        const next = { x: offset.x * 0.78, y: offset.y * 0.78 };
        if (Math.hypot(next.x, next.y) > 0.25) {
          nextClusterOffsets[clusterId] = next;
          moving = true;
        }
      }
      panOffsetRef.current = moving ? nextPan : { x: 0, y: 0 };
      dragOffsetsRef.current = moving ? nextOffsets : {};
      clusterOffsetsRef.current = moving ? nextClusterOffsets : {};
      setPanOffset(panOffsetRef.current);
      setDragOffsets(dragOffsetsRef.current);
      setClusterOffsets(clusterOffsetsRef.current);
      if (moving) {
        returnAnimationRef.current = requestAnimationFrame(step);
      }
    };
    returnAnimationRef.current = requestAnimationFrame(step);
  };

  const graphTransform = `translate(${graphViewport.center.x} ${graphViewport.center.y}) scale(${zoom}) translate(${-graphViewport.center.x} ${-graphViewport.center.y}) translate(${panOffset.x} ${panOffset.y})`;

  return (
    <section
      className="skill-graph"
      aria-label={`${viewLabels[activeView]} Skills 关系图`}
    >
      <svg
        ref={svgRef}
        className="skill-graph__canvas"
        viewBox={`0 0 ${graphViewport.width} ${graphViewport.height}`}
        role="img"
        aria-label={`${viewLabels[activeView]} Skills 径向关系图`}
        onWheel={onWheel}
        onPointerDown={beginCanvasDrag}
        onPointerMove={moveDrag}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
      >
        <g transform={graphTransform}>
          <g className="skill-graph__clusters">
            {(graph?.clusters ?? []).map((cluster) => {
              const point = clusterPoint(cluster.clusterId);
              const geometry = calculateDynamicClusterGeometry(
                cluster.memberSkillIds.map(displayPoint),
                point,
              );
              return (
                <g
                  key={cluster.clusterId}
                  className={`skill-graph__cluster${cluster.semanticStatus !== "ready" ? " is-semantic-stale" : ""}${selectedCluster && selectedCluster.clusterId !== cluster.clusterId ? " is-muted" : ""}`}
                >
                  <circle className="skill-graph__cluster-cloud" cx={geometry.x} cy={geometry.y} r={geometry.radius} />
                  {cluster.coreSkillIds.map((skillId) => {
                    const target = displayPoint(skillId);
                    return <line key={skillId} className="skill-graph__core-link" x1={point.x} y1={point.y} x2={target.x} y2={target.y} />;
                  })}
                  <g
                    className="skill-graph__cluster-center"
                    transform={`translate(${point.x} ${point.y})`}
                    tabIndex={0}
                    role="button"
                    aria-label={`${cluster.name} 集群`}
                    onPointerDown={(event) => beginClusterDrag(event, cluster.clusterId)}
                    onDoubleClick={(event) => {
                      event.stopPropagation();
                      setSelection({ type: "cluster", clusterId: cluster.clusterId });
                    }}
                    onKeyDown={(event) => {
                      if (event.key !== "Enter" && event.key !== " ") return;
                      event.preventDefault();
                      setSelection({ type: "cluster", clusterId: cluster.clusterId });
                    }}
                  >
                    <circle r="17" />
                    <text textAnchor="middle" dominantBaseline="middle">{shortLabel(cluster.name, 10)}</text>
                  </g>
                </g>
              );
            })}
          </g>
          <g className="skill-graph__edges">
            {(graph?.edges ?? []).map((edge) => (
              <GraphEdge
                key={edge.edgeId}
                edge={edge}
                source={displayPoint(edge.sourceSkillId)}
                target={displayPoint(edge.targetSkillId)}
                expanded={selectedEdge?.edgeId === edge.edgeId}
                className={edgeFocusClass(edge)}
                onDoubleClick={(event) => {
                  event.stopPropagation();
                  setSelection({ type: "edge", edgeId: edge.edgeId });
                }}
              />
            ))}
          </g>

          <g className="skill-graph__nodes">
            {(graph?.nodes ?? []).map((node, index) => {
              const point = displayPoint(node.skillId);
              return (
                <g
                  key={node.skillId}
                  className={`skill-graph__node${node.superseded ? " is-superseded" : ""}${node.disabled ? " is-disabled" : ""}${node.classificationStatus !== "ready" ? " is-classification-stale" : ""}${selectedNode && selectedNode.skillId !== node.skillId ? " is-muted" : ""}${selectedCluster && node.clusterId !== selectedCluster.clusterId ? " is-muted" : ""}`}
                  transform={`translate(${point.x} ${point.y})`}
                  tabIndex={0}
                  role="button"
                  aria-label={`${node.name} Skill`}
                  onPointerDown={(event) => beginNodeDrag(event, node.skillId)}
                  onDoubleClick={(event) => {
                    event.stopPropagation();
                    setSelection({ type: "node", nodeId: node.skillId });
                  }}
                  onKeyDown={(event) => {
                    if (event.key !== "Enter" && event.key !== " ") return;
                    event.preventDefault();
                    setSelection({ type: "node", nodeId: node.skillId });
                  }}
                >
                  <g
                    className="skill-graph__node-motion"
                    style={{
                      animationDelay: `${-(index % 11) * 0.37}s`,
                      animationDuration: `${5.2 + (index % 7) * 0.31}s`,
                    }}
                  >
                    <circle className="skill-graph__node-halo" r="18" />
                    <circle className="skill-graph__node-dot" r="10" />
                    <text y="27" textAnchor="middle">
                      {shortLabel(node.name)}
                    </text>
                  </g>
                </g>
              );
            })}
          </g>

          {selectedEdge && (
            <EdgePopup
              edge={selectedEdge}
              source={displayPoint(selectedEdge.sourceSkillId)}
              target={displayPoint(selectedEdge.targetSkillId)}
              onPointerDown={(event) => event.stopPropagation()}
            />
          )}
          {selectedNode && (
            <NodePopup
              node={selectedNode}
              point={displayPoint(selectedNode.skillId)}
              onPointerDown={(event) => event.stopPropagation()}
            />
          )}
          {selectedCluster && (
            <ClusterPopup
              cluster={selectedCluster}
              point={clusterPoint(selectedCluster.clusterId)}
              onPointerDown={(event) => event.stopPropagation()}
            />
          )}
        </g>
      </svg>

      <div className="skill-graph__status" aria-live="polite">
        {state === "loading" && "正在读取向量关系…"}
        {state === "error" && `关系图读取失败：${error}`}
        {state === "ready" &&
          !isNative &&
          "请在桌面窗口中查看向量关系图"}
        {state === "ready" &&
          isNative &&
          graph &&
          `${graph.nodes.length} 个节点 · ${graph.edges.length} 条边${
            graph.excludedUnreadyCount + graph.excludedUnconnectedCount > 0
              ? ` · ${
                  graph.excludedUnreadyCount + graph.excludedUnconnectedCount
                } 项未出图`
              : ""
          }`}
      </div>
      {state === "ready" &&
        graph &&
        graph.nodes.length === 0 &&
        isNative && (
          <p className="skill-graph__empty">
            {graph.excludedUnreadyCount > 0
              ? "当前类别尚无可显示的就绪向量；请先更新活动 Embedding Profile"
              : "当前类别尚无可显示的实体 Skill 向量"}
          </p>
        )}
    </section>
  );
}

function GraphEdge({
  edge,
  source,
  target,
  expanded,
  className,
  onDoubleClick,
}: {
  edge: SkillGraphEdge;
  source: GraphPoint;
  target: GraphPoint;
  expanded: boolean;
  className: string;
  onDoubleClick: (event: ReactPointerEvent<SVGLineElement>) => void;
}) {
  const lineCount = expanded ? Math.max(1, edge.relations.length) : 1;
  const dx = target.x - source.x;
  const dy = target.y - source.y;
  const distance = Math.max(1, Math.hypot(dx, dy));
  const normal = { x: -dy / distance, y: dx / distance };
  return (
    <g className={`skill-graph__edge ${expanded ? "is-expanded" : ""} ${className}`}>
      {Array.from({ length: lineCount }, (_, index) => {
        const offset = parallelEdgeOffset(index, lineCount);
        const coordinates = {
          x1: source.x + normal.x * offset,
          y1: source.y + normal.y * offset,
          x2: target.x + normal.x * offset,
          y2: target.y + normal.y * offset,
        };
        return (
          <g key={`${edge.edgeId}:${index}`}>
            <line className="skill-graph__edge-line" {...coordinates} />
            <line className="skill-graph__edge-flow" {...coordinates} />
            <line
              className="skill-graph__edge-hit"
              {...coordinates}
              onDoubleClick={onDoubleClick}
            />
          </g>
        );
      })}
    </g>
  );
}

function EdgePopup({
  edge,
  source,
  target,
  onPointerDown,
}: {
  edge: SkillGraphEdge;
  source: GraphPoint;
  target: GraphPoint;
  onPointerDown: (event: ReactPointerEvent<SVGForeignObjectElement>) => void;
}) {
  const x = (source.x + target.x) / 2;
  const y = (source.y + target.y) / 2;
  const popupX = Math.min(graphViewport.width - 244, Math.max(12, x + 12));
  const popupY = Math.min(graphViewport.height - 170, Math.max(12, y - 20));
  return (
    <foreignObject
      className="skill-graph__popup-shell"
      x={popupX}
      y={popupY}
      width="232"
      height="158"
      onPointerDown={onPointerDown}
    >
      <div className="skill-graph__popup">
        <strong>具体关系</strong>
        <ul>
          {edge.relations.map((relation, index) => (
            <li key={`${relation.relationshipType}:${relation.state}:${index}`}>
              <span>{relationLabel(relation)}</span>
              <small>{stateLabels[relation.state] ?? relation.state}</small>
            </li>
          ))}
        </ul>
      </div>
    </foreignObject>
  );
}

function NodePopup({
  node,
  point,
  onPointerDown,
}: {
  node: SkillGraphNode;
  point: GraphPoint;
  onPointerDown: (event: ReactPointerEvent<SVGForeignObjectElement>) => void;
}) {
  const popupX = Math.min(
    graphViewport.width - 274,
    Math.max(12, point.x + 16),
  );
  const popupY = Math.min(
    graphViewport.height - 190,
    Math.max(12, point.y - 30),
  );
  return (
    <foreignObject
      className="skill-graph__popup-shell"
      x={popupX}
      y={popupY}
      width="262"
      height="178"
      onPointerDown={onPointerDown}
    >
      <div className="skill-graph__popup">
        <strong>{node.name}</strong>
        <p>{node.description || "暂无简介"}</p>
        <small>{node.enabledAgents.join(" · ")}</small>
        {node.disabled && <small>当前 Agent 中已禁用</small>}
        {node.classificationStatus !== "ready" && <small>分类已过期，等待增量更新重试</small>}
        <code title={node.path}>{node.path}</code>
      </div>
    </foreignObject>
  );
}

function ClusterPopup({
  cluster,
  point,
  onPointerDown,
}: {
  cluster: SkillGraphCluster;
  point: GraphPoint;
  onPointerDown: (event: ReactPointerEvent<SVGForeignObjectElement>) => void;
}) {
  const popupX = Math.min(graphViewport.width - 274, Math.max(12, point.x + 20));
  const popupY = Math.min(graphViewport.height - 170, Math.max(12, point.y - 30));
  return (
    <foreignObject
      className="skill-graph__popup-shell"
      x={popupX}
      y={popupY}
      width="262"
      height="158"
      onPointerDown={onPointerDown}
    >
      <div className="skill-graph__popup">
        <strong>{cluster.name}</strong>
        <p>{cluster.summary}</p>
        <small>{cluster.memberSkillIds.length} 个 Skills</small>
        {cluster.semanticStatus !== "ready" && <small>待生成：下次增量更新将重试</small>}
      </div>
    </foreignObject>
  );
}

function relationLabel(relation: SkillGraphRelation): string {
  const base =
    relationLabels[relation.relationshipType] ?? relation.relationshipType;
  if (relation.source === "vector_similarity") {
    return relation.state === "nearest_fallback"
      ? `${base}（最近无冲突）`
      : `${base}（向量过阈）`;
  }
  return base;
}

function shortLabel(value: string, limit = 18): string {
  return value.length > limit ? `${value.slice(0, limit - 1)}…` : value;
}
