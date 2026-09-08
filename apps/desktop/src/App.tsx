import {
  type CSSProperties,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { api, isNativeRuntime as detectNativeRuntime } from "./api";
import { filterCanonicalSkills } from "./canonical";
import { SemanticSearch } from "./components/SemanticSearch";
import { SettingsDialog } from "./components/SettingsDialog";
import { SkillGraph } from "./components/SkillGraph";
import type {
  AgentId,
  AgentSkillsResponse,
  CanonicalSnapshot,
  InstalledSkill,
  ProgressEvent,
  SkillActionPlan,
  ViewId,
} from "./types";
import "./App.css";

const agents: Array<{ id: AgentId; label: string; shortLabel: string }> = [
  { id: "claude-code", label: "Claude Code", shortLabel: "Claude" },
  { id: "cursor", label: "Cursor", shortLabel: "Cursor" },
  { id: "codex", label: "Codex", shortLabel: "Codex" },
];

function App() {
  const isNativeRuntime = detectNativeRuntime();
  const [isKeyDialogOpen, setIsKeyDialogOpen] = useState(false);
  const [isAssistantOpen, setIsAssistantOpen] = useState(false);
  const [activeView, setActiveView] = useState<ViewId | null>(null);
  const [isSkillsListCollapsed, setIsSkillsListCollapsed] = useState(false);
  const [skillResult, setSkillResult] = useState<AgentSkillsResponse | null>(
    null,
  );
  const [skillState, setSkillState] = useState<
    "idle" | "loading" | "ready" | "error"
  >("idle");
  const [skillError, setSkillError] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [agentFilter, setAgentFilter] = useState<"all" | AgentId>("all");
  const [libraryVersion, setLibraryVersion] = useState(0);
  const [graphRevision, setGraphRevision] = useState(0);
  const [skillAction, setSkillAction] = useState<{
    path: string | null;
    message: string;
    type: "idle" | "success" | "error";
  }>({ path: null, message: "", type: "idle" });
  const [installTargets, setInstallTargets] = useState<
    Record<string, AgentId>
  >({});
  const [expandedSkills, setExpandedSkills] = useState<Set<string>>(
    () => new Set(),
  );
  const [skillActionDialog, setSkillActionDialog] = useState<{
    plan?: SkillActionPlan;
    skill?: InstalledSkill;
    loading: boolean;
    error?: string;
  } | null>(null);
  const [copyFailureDialog, setCopyFailureDialog] = useState<{
    skillName: string;
    targetName: string;
    message: string;
  } | null>(null);
  const keyInputRef = useRef<HTMLInputElement>(null);
  const dButtonRef = useRef<HTMLButtonElement>(null);
  const skillRefreshInFlight = useRef(false);
  const suppressSkillEventsUntil = useRef(0);

  const b2Width = Math.round(window.screen.width * 0.15);
  const shellStyle = {
    "--skills-width": !activeView
      ? "0px"
      : isSkillsListCollapsed
        ? "var(--skills-collapse-width)"
        : `${b2Width}px`,
  } as CSSProperties;

  useEffect(() => {
    if (!isKeyDialogOpen) return;
    requestAnimationFrame(() => keyInputRef.current?.focus());
  }, [isKeyDialogOpen]);

  const loadSkills = useCallback(async (refresh: boolean) => {
    const snapshot: CanonicalSnapshot = await (refresh
      ? api.refreshCanonicalSkillsSnapshot()
      : api.getCanonicalSkillsSnapshot());
    return {
      agent: "all" as const,
      officialDocumentation: "",
      searchedPaths: snapshot.searchedPaths,
      skills: snapshot.skills,
      warnings: snapshot.warnings,
    };
  }, []);

  useEffect(() => {
    if (!isNativeRuntime) {
      if (activeView) {
        setSkillResult(null);
        setSkillError(
          "浏览器预览无法读取本机目录，请在 Tauri 桌面窗口中使用此功能。",
        );
        setSkillState("error");
      }
      return;
    }

    let isCurrent = true;
    setSkillState("loading");
    setSkillError("");
    const refreshing = libraryVersion > 0;
    if (refreshing) skillRefreshInFlight.current = true;
    loadSkills(refreshing)
      .then((result) => {
        if (!isCurrent) return;
        setSkillResult(result);
        setSkillState("ready");
      })
      .catch((error) => {
        if (!isCurrent) return;
        setSkillError(String(error));
        setSkillState("error");
      })
      .finally(() => {
        if (!refreshing) return;
        suppressSkillEventsUntil.current = Date.now() + 3000;
        window.setTimeout(() => {
          skillRefreshInFlight.current = false;
        }, 3000);
      });

    return () => {
      isCurrent = false;
    };
  }, [isNativeRuntime, libraryVersion, loadSkills]);

  useEffect(() => {
    if (!isNativeRuntime) return;
    let disposed = false;
    let stopListening: (() => void) | undefined;
    let refreshTimer: number | undefined;
    listen("all-skills-changed", () => {
      if (
        skillRefreshInFlight.current ||
        Date.now() < suppressSkillEventsUntil.current
      ) {
        return;
      }
      window.clearTimeout(refreshTimer);
      refreshTimer = window.setTimeout(() => {
        skillRefreshInFlight.current = true;
        setLibraryVersion((version) => version + 1);
      }, 1000);
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        stopListening = unlisten;
      }
    });
    return () => {
      disposed = true;
      window.clearTimeout(refreshTimer);
      stopListening?.();
    };
  }, [isNativeRuntime]);

  useEffect(() => {
    if (!isNativeRuntime) return;
    let disposed = false;
    let stopListening: (() => void) | undefined;
    listen<ProgressEvent>("embedding-job-progress", ({ payload }) => {
      if (payload.total > 0 && payload.completed >= payload.total) {
        setGraphRevision((revision) => revision + 1);
      }
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stopListening = unlisten;
    });
    return () => {
      disposed = true;
      stopListening?.();
    };
  }, [isNativeRuntime]);

  const closeKeyDialog = () => {
    setIsKeyDialogOpen(false);
    setGraphRevision((revision) => revision + 1);
    requestAnimationFrame(() => dButtonRef.current?.focus());
  };

  const toggleView = (view: ViewId) => {
    const closing = activeView === view;
    const openingFromIdle = activeView === null;
    setActiveView(closing ? null : view);
    if (closing || openingFromIdle) {
      setIsSkillsListCollapsed(false);
    }
    setSearchQuery("");
    setAgentFilter("all");
    setSkillAction({ path: null, message: "", type: "idle" });
  };


  const toggleSkillExpansion = (path: string) => {
    setExpandedSkills((current) => {
      const next = new Set(current);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };

  const visibleSkills = useMemo(() => {
    return filterCanonicalSkills(skillResult?.skills ?? [], {
      activeView,
      agentFilter,
      searchQuery,
    });
  }, [activeView, agentFilter, searchQuery, skillResult]);

  const runSkillAction = async (
    path: string,
    action: () => Promise<unknown>,
    successMessage: string,
    onError?: (message: string) => void,
  ) => {
    setSkillAction({ path, message: "", type: "idle" });
    try {
      await action();
      setSkillAction({ path: null, message: successMessage, type: "success" });
      setLibraryVersion((version) => version + 1);
    } catch (error) {
      const message = String(error);
      setSkillAction({
        path: null,
        message,
        type: "error",
      });
      onError?.(message);
    }
  };

  const installToAgent = (skill: InstalledSkill) => {
    if (!skill.libraryPath) return;
    const firstAvailable = agents.find(
      (agent) => !skill.enabledAgents.includes(agent.id),
    )?.id;
    const target = installTargets[skill.path] ?? firstAvailable;
    if (!target) {
      setCopyFailureDialog({
        skillName: skill.name,
        targetName: "目标 Agent",
        message: "没有可用的目标 Agent，或该 Skill 已存在于所有支持的 Agent 中。",
      });
      return;
    }
    const targetName = agents.find((agent) => agent.id === target)?.label ?? target;
    if (!window.confirm(`将 ${skill.name} 复制到 ${targetName} 吗？`)) {
      return;
    }
    runSkillAction(
      skill.path,
      () => api.copyLibrarySkillToAgent(skill.libraryPath!, target),
      `${skill.name} 已复制到 ${targetName}。`,
      (message) => setCopyFailureDialog({
        skillName: skill.name,
        targetName,
        message,
      }),
    );
  };

  const prepareSkillAction = async (
    skill: InstalledSkill,
    action: "uninstall" | "disable",
  ) => {
    if (!activeView) return;
    setSkillActionDialog({ skill, loading: true });
    try {
      const plan = await api.prepareSkillAction(skill.skillId, activeView, action);
      setSkillActionDialog({ skill, plan, loading: false });
    } catch (error) {
      setSkillActionDialog({ skill, loading: false, error: String(error) });
    }
  };

  const executePreparedAction = async () => {
    const dialog = skillActionDialog;
    if (!dialog?.plan?.allowed || !dialog.skill) return;
    const { plan, skill } = dialog;
    setSkillActionDialog({ ...dialog, loading: true });
    await runSkillAction(
      skill.path,
      () =>
        plan.action === "uninstall"
          ? api.uninstallSkill(skill.skillId, plan.viewId)
          : api.disableSkillForAgent(skill.skillId, plan.viewId),
      plan.action === "uninstall"
        ? `${skill.name} 已从 ${plan.viewId === "all" ? "所有 Skills" : plan.viewId} 卸载。`
        : `${skill.name} 已在当前 Agent 中禁用。`,
    );
    setSkillActionDialog(null);
  };

  const disableInstead = async () => {
    const skill = skillActionDialog?.skill;
    const viewId = skillActionDialog?.plan?.viewId;
    if (!skill || !viewId || viewId === "all") return;
    setSkillActionDialog({ skill, loading: true });
    try {
      const plan = await api.prepareSkillAction(skill.skillId, viewId, "disable");
      setSkillActionDialog({ skill, plan, loading: false });
    } catch (error) {
      setSkillActionDialog({ skill, loading: false, error: String(error) });
    }
  };

  const enableSkill = (skill: InstalledSkill, agent: AgentId) => {
    runSkillAction(
      skill.path,
      () => api.enableSkillForAgent(skill.skillId, agent),
      `${skill.name} 已在 ${agents.find((item) => item.id === agent)?.label} 中解禁。`,
    );
  };

  const restoreBackup = (skill: InstalledSkill) => {
    runSkillAction(
      skill.path,
      () => api.restoreSkillBackup(skill.skillId),
      `${skill.name} 已重新加入所有 Skills 后备库。`,
    );
  };

  return (
    <div
      className={`app-shell ${activeView ? "app-shell--skills-open" : ""} ${
        activeView && isSkillsListCollapsed ? "app-shell--skills-collapsed" : ""
      } ${isAssistantOpen ? "app-shell--assistant-open" : ""}`}
      data-page="a1"
      style={shellStyle}
    >
      <aside className="side-platform" aria-label="左侧平台">
        <button
          ref={dButtonRef}
          className="brand-mark"
          type="button"
          aria-label="打开 API Key 管理"
          title="API Key 管理"
          onClick={() => setIsKeyDialogOpen(true)}
        >
          <span>D</span>
        </button>

        <div className="agent-actions" aria-label="Agent 平台">
          <button
            className={`agent-button agent-button--all ${
              activeView === "all" ? "agent-button--active" : ""
            }`}
            type="button"
            aria-label="所有 Skills"
            aria-pressed={activeView === "all"}
            title="所有 Skills"
            onClick={() => toggleView("all")}
          >
            <svg
              className="folder-icon"
              viewBox="0 0 24 24"
              aria-hidden="true"
            >
              <path
                d="M3.75 6.75A1.75 1.75 0 0 1 5.5 5h4.1c.5 0 .97.21 1.3.58l1.05 1.17h6.55a1.75 1.75 0 0 1 1.75 1.75v8A1.75 1.75 0 0 1 18.5 18h-13a1.75 1.75 0 0 1-1.75-1.75v-9.5Z"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinejoin="round"
              />
            </svg>
            <span className="agent-button__full">所有 Skills</span>
            <span className="agent-button__short">全部</span>
          </button>
          {agents.map((agent) => (
            <button
              key={agent.id}
              className={`agent-button ${
                activeView === agent.id ? "agent-button--active" : ""
              }`}
              type="button"
              aria-label={`${agent.label} Skills`}
              aria-pressed={activeView === agent.id}
              title={`${agent.label} Skills`}
              onClick={() => toggleView(agent.id)}
            >
              <span className="agent-button__full">{agent.label}</span>
              <span className="agent-button__short">{agent.shortLabel}</span>
            </button>
          ))}
          <button
            className="agent-button agent-button--add"
            type="button"
            aria-label="添加（功能待定）"
            title="功能待定"
            disabled
          >
            +
          </button>
        </div>

        <p className="page-code">A1</p>
      </aside>

      <aside
        className={`skills-platform${
          isSkillsListCollapsed ? " skills-platform--collapsed" : ""
        }`}
        aria-label="Agent Skills"
        aria-hidden={!activeView}
      >
        {activeView && (
          <>
            <header className="skills-header">
              {!isSkillsListCollapsed && (
                <div>
                  <p className="eyebrow">
                    {activeView === "all" ? "SKILL LIBRARY" : "AGENT SKILLS"}
                  </p>
                  <h2>
                    {activeView === "all"
                      ? "所有 Skills"
                      : agents.find((agent) => agent.id === activeView)?.label}
                  </h2>
                </div>
              )}
              <div className="skills-header__actions">
                {!isSkillsListCollapsed && (
                  <span className="skill-count">
                    {skillState === "ready" ? visibleSkills.length : "—"}
                  </span>
                )}
                <button
                  className="skills-collapse-toggle"
                  type="button"
                  aria-label={
                    isSkillsListCollapsed
                      ? "展开 Skills 列表"
                      : "收起 Skills 列表"
                  }
                  aria-expanded={!isSkillsListCollapsed}
                  title={
                    isSkillsListCollapsed
                      ? "展开 Skills 列表"
                      : "收起 Skills 列表"
                  }
                  onClick={() =>
                    setIsSkillsListCollapsed((current) => !current)
                  }
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path
                      d={
                        isSkillsListCollapsed
                          ? "M9 6l6 6-6 6"
                          : "M15 6l-6 6 6 6"
                      }
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="1.5"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    />
                  </svg>
                </button>
              </div>
            </header>

            {!isSkillsListCollapsed && (
              <>
            <div className="skills-content">
              {activeView === "all" && (
                <div className="all-skills-tools">
                  <label className="skill-search">
                    <span className="sr-only">搜索 Skills</span>
                    <input
                      type="search"
                      value={searchQuery}
                      placeholder="搜索名称或描述"
                      onChange={(event) =>
                        setSearchQuery(event.currentTarget.value)
                      }
                    />
                  </label>
                  <div className="agent-filters" aria-label="按 Agent 筛选">
                    <button
                      type="button"
                      className={agentFilter === "all" ? "is-active" : ""}
                      onClick={() => setAgentFilter("all")}
                    >
                      全部
                    </button>
                    {agents.map((agent) => (
                      <button
                        key={agent.id}
                        type="button"
                        className={
                          agentFilter === agent.id ? "is-active" : ""
                        }
                        onClick={() => setAgentFilter(agent.id)}
                      >
                        {agent.shortLabel}
                      </button>
                    ))}
                  </div>
                </div>
              )}
              {skillAction.message && (
                <div
                  className={`skill-action-feedback skill-action-feedback--${skillAction.type}`}
                  role="status"
                >
                  {skillAction.message}
                </div>
              )}
              {skillResult && skillResult.warnings.length > 0 && (
                <div className="skill-warning" role="status">
                  {activeView === "all"
                    ? `${skillResult.warnings.length} 个来源或条目读取失败，其余结果已保留。`
                    : skillResult.warnings.join("；")}
                </div>
              )}
              {skillState === "loading" && (
                <div className="skills-state">
                  <span className="spinner" />
                  <p>正在读取官方目录…</p>
                </div>
              )}
              {skillState === "error" && (
                <div className="skills-state skills-state--error">
                  <p>读取失败</p>
                  <span>{skillError}</span>
                </div>
              )}
              {skillState === "ready" &&
                skillResult &&
                visibleSkills.length === 0 && (
                  <div className="skills-state">
                    <p>
                      {skillResult.skills.length === 0
                        ? "尚未发现 Skills"
                        : "没有匹配的 Skills"}
                    </p>
                    <span>
                      {activeView === "all"
                        ? "当前统一快照中没有匹配项"
                        : "当前统一快照中该 Agent 没有匹配项"}
                    </span>
                  </div>
                )}
              {skillState === "ready" &&
                skillResult &&
                visibleSkills.length > 0 && (
                  <ul className="skill-list">
                    {visibleSkills.map((skill) => {
                      const isExpanded = expandedSkills.has(skill.path);
                      const activeAgent = activeView !== "all" ? activeView : undefined;
                      const isDisabled = Boolean(
                        activeAgent && skill.disabledAgents.includes(activeAgent),
                      );
                      const availableAgents = agents.filter(
                        (agent) => !skill.enabledAgents.includes(agent.id),
                      );
                      return (
                      <li
                        key={skill.skillId}
                        className={`skill-item ${
                          isExpanded ? "skill-item--expanded" : ""
                        }${isDisabled ? " skill-item--disabled" : ""}`}
                      >
                        <span className="skill-item__mark" />
                        <div className="skill-item__body">
                          <div className="skill-item__title">
                            <div className="skill-item__identity">
                              <p>{skill.name}</p>
                              {skill.isBuiltIn && (
                                <span className="skill-item__badge">内置</span>
                              )}
                              {isDisabled && (
                                <span className="skill-item__badge skill-item__badge--disabled">已禁用</span>
                              )}
                            </div>
                            <div className="skill-item__quick-actions">
                              <button
                                className="skill-quick-action skill-quick-action--uninstall"
                                type="button"
                                aria-label={`${activeView === "all" ? "删除后备副本" : "卸载"} ${skill.name}`}
                                title={activeView === "all" ? "删除后备副本" : "从当前 Agent 卸载"}
                                disabled={skillAction.path === skill.path || (activeView === "all" && !skill.inLibrary)}
                                onClick={() => prepareSkillAction(skill, "uninstall")}
                              >
                                ×
                              </button>
                              {activeView !== "all" && activeAgent && (
                                <button
                                  className={`skill-quick-action ${isDisabled ? "skill-quick-action--enable" : "skill-quick-action--disable"}`}
                                  type="button"
                                  aria-label={`${isDisabled ? "解禁" : "禁用当前 Agent 中的"} ${skill.name}`}
                                  title={isDisabled ? "解禁" : "禁用当前"}
                                  disabled={skillAction.path === skill.path}
                                  onClick={() =>
                                    isDisabled
                                      ? enableSkill(skill, activeAgent)
                                      : prepareSkillAction(skill, "disable")
                                  }
                                >
                                  {isDisabled ? "✓" : "−"}
                                </button>
                              )}
                              {activeView === "all" && skill.backupSuppressed && (
                                <button
                                  className="skill-quick-action skill-quick-action--restore"
                                  type="button"
                                  aria-label={`重新备份 ${skill.name}`}
                                  title="从 Agent 重新添加到所有 Skills"
                                  disabled={skillAction.path === skill.path}
                                  onClick={() => restoreBackup(skill)}
                                >
                                  ↥
                                </button>
                              )}
                            </div>
                          </div>
                          {skill.description && <span>{skill.description}</span>}
                          {activeView !== "all" && (
                            <code title={skill.path}>{skill.path}</code>
                          )}
                          <button
                            className="skill-expand-button"
                            type="button"
                            aria-expanded={isExpanded}
                            aria-label={`${isExpanded ? "收起" : "展开"} ${
                              skill.name
                            } 的完整信息`}
                            onClick={() => toggleSkillExpansion(skill.path)}
                          >
                            {isExpanded ? "收起" : "展开"}
                          </button>
                          {activeView === "all" && isExpanded && (
                            <div className="skill-library-actions">
                              {skill.inLibrary &&
                                skill.libraryPath &&
                                availableAgents.length > 0 && (
                                  <div className="skill-install-control">
                                    <select
                                      aria-label={`选择 ${skill.name} 的安装目标`}
                                      value={
                                        installTargets[skill.path] ??
                                        availableAgents[0].id
                                      }
                                      onChange={(event) => {
                                        const target = event.currentTarget.value as AgentId;
                                        setInstallTargets((current) => ({
                                          ...current,
                                          [skill.path]: target,
                                        }));
                                      }}
                                    >
                                      {availableAgents.map((agent) => (
                                        <option
                                          key={agent.id}
                                          value={agent.id}
                                        >
                                          {agent.shortLabel}
                                        </option>
                                      ))}
                                    </select>
                                    <button
                                      type="button"
                                      disabled={
                                        skillAction.path === skill.path
                                      }
                                      onClick={() => installToAgent(skill)}
                                    >
                                      复制到 Agent
                                    </button>
                                  </div>
                                )}
                            </div>
                          )}
                        </div>
                      </li>
                    )})}
                  </ul>
                )}
            </div>

            {skillResult && (
              <footer className="skills-footer">
                <span>统一 All Skills 快照</span>
                <span title={skillResult.searchedPaths.join("\n")}>
                  {skillResult.searchedPaths.length} 个扫描目录
                </span>
              </footer>
            )}
              </>
            )}
          </>
        )}
      </aside>

      {skillActionDialog && (
        <div className="skill-action-backdrop" role="presentation" onMouseDown={() => !skillActionDialog.loading && setSkillActionDialog(null)}>
          <section
            className="skill-action-dialog"
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="skill-action-title"
            onMouseDown={(event) => event.stopPropagation()}
          >
            <header>
              <div>
                <p className="eyebrow">SKILL CONTROL</p>
                <h3 id="skill-action-title">
                  {skillActionDialog.plan?.action === "disable" ? "禁用当前 Agent" : "确认卸载"}
                </h3>
              </div>
              <button type="button" className="modal-close" aria-label="关闭" disabled={skillActionDialog.loading} onClick={() => setSkillActionDialog(null)}>×</button>
            </header>
            {skillActionDialog.loading && <div className="skill-action-dialog__state"><span className="spinner" />正在核对实际影响范围…</div>}
            {skillActionDialog.error && <p className="skill-action-dialog__error">{skillActionDialog.error}</p>}
            {skillActionDialog.plan && !skillActionDialog.loading && (
              <>
                <strong>{skillActionDialog.plan.skillName}</strong>
                <p>
                  {skillActionDialog.plan.action === "disable"
                    ? `只禁用 ${skillActionDialog.plan.viewId}，不会删除 Skill 文件或向量。`
                    : skillActionDialog.plan.viewId === "all"
                      ? "后备副本将被删除，并停止在后续扫描中自动重新备份。"
                      : `将从 ${skillActionDialog.plan.viewId} 的实际安装位置卸载。`}
                </p>
                {skillActionDialog.plan.sharedInstallation && (
                  <div className="skill-action-dialog__impact">
                    此路径同时被以下 Agent 使用：{skillActionDialog.plan.affectedAgents.join("、")}。继续卸载会同时影响它们。
                  </div>
                )}
                {skillActionDialog.plan.pluginOperation && (
                  <div className="skill-action-dialog__impact">该 Skill 由插件提供，卸载可能同时影响插件内其他 Skills。</div>
                )}
                {skillActionDialog.plan.symbolicLinkOnly && (
                  <div className="skill-action-dialog__note">只会移除符号链接，不会删除链接目标内容。</div>
                )}
                {skillActionDialog.plan.targetPaths.length > 0 && (
                  <details>
                    <summary>实际目标 · {skillActionDialog.plan.targetPaths.length}</summary>
                    {skillActionDialog.plan.targetPaths.map((path) => <code key={path}>{path}</code>)}
                  </details>
                )}
                {!skillActionDialog.plan.allowed && (
                  <p className="skill-action-dialog__error">{skillActionDialog.plan.reason}</p>
                )}
                <footer>
                  {skillActionDialog.plan.action === "uninstall" && skillActionDialog.plan.viewId !== "all" && (
                    <button type="button" className="warning-button" onClick={disableInstead}>禁用当前</button>
                  )}
                  <button type="button" onClick={() => setSkillActionDialog(null)}>取消</button>
                  <button
                    type="button"
                    className={skillActionDialog.plan.action === "uninstall" ? "danger-button" : "warning-button"}
                    disabled={!skillActionDialog.plan.allowed}
                    onClick={executePreparedAction}
                  >
                    {skillActionDialog.plan.action === "uninstall" ? "确认卸载" : "确认禁用"}
                  </button>
                </footer>
              </>
            )}
          </section>
        </div>
      )}

      {copyFailureDialog && (
        <div
          className="skill-action-backdrop"
          role="presentation"
          onMouseDown={() => setCopyFailureDialog(null)}
        >
          <section
            className="skill-action-dialog copy-failure-dialog"
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="copy-failure-title"
            onMouseDown={(event) => event.stopPropagation()}
          >
            <header>
              <div>
                <p className="eyebrow">SKILL COPY</p>
                <h3 id="copy-failure-title">复制失败</h3>
              </div>
              <button
                type="button"
                className="modal-close"
                aria-label="关闭复制失败提醒"
                onClick={() => setCopyFailureDialog(null)}
              >
                ×
              </button>
            </header>
            <strong>{copyFailureDialog.skillName}</strong>
            <p>未能复制到 {copyFailureDialog.targetName}。</p>
            <div className="skill-action-dialog__impact copy-failure-dialog__reason">
              {copyFailureDialog.message}
            </div>
            <p className="copy-failure-dialog__hint">
              可能原因包括目标已存在、Skill 包结构无效、包含不可复制的链接、目录权限不足，或目标 Agent 不兼容。
            </p>
            <footer>
              <button type="button" onClick={() => setCopyFailureDialog(null)}>
                知道了
              </button>
            </footer>
          </section>
        </div>
      )}

      <main className="main-platform" aria-label="A1 可视化工作区">
        {activeView && (
          <SkillGraph
            activeView={activeView}
            isNative={isNativeRuntime}
            refreshKey={graphRevision + libraryVersion}
          />
        )}
      </main>

      <aside
        className="assistant-platform"
        aria-label="B3 语义搜索平台"
      >
        <div
          className="assistant-platform__content"
          aria-hidden={!isAssistantOpen}
        >
          <SemanticSearch
            isNative={isNativeRuntime}
            skills={skillResult?.skills ?? []}
          />
        </div>
        <div className="assistant-platform__rail">
          <button
            className="assistant-toggle"
            type="button"
            aria-label={isAssistantOpen ? "收起语义搜索" : "展开语义搜索"}
            aria-expanded={isAssistantOpen}
            title={isAssistantOpen ? "收起语义搜索" : "展开语义搜索"}
            onClick={() => setIsAssistantOpen((current) => !current)}
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path
                d="M5.25 4.75h13.5A2.25 2.25 0 0 1 21 7v8.25a2.25 2.25 0 0 1-2.25 2.25h-7.1L7.2 20.6a.75.75 0 0 1-1.18-.61V17.5h-.77A2.25 2.25 0 0 1 3 15.25V7a2.25 2.25 0 0 1 2.25-2.25Z"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinejoin="round"
              />
              <path
                d="M7.5 9h9M7.5 13h6"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinecap="round"
              />
            </svg>
          </button>
        </div>
      </aside>

      {isKeyDialogOpen && (
        <SettingsDialog
          isNative={isNativeRuntime}
          keyInputRef={keyInputRef}
          onClose={closeKeyDialog}
        />
      )}
    </div>
  );
}

export default App;
