import {
  type CSSProperties,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { invoke } from "@tauri-apps/api/core";
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
  ) => {
    setSkillAction({ path, message: "", type: "idle" });
    try {
      await action();
      setSkillAction({ path: null, message: successMessage, type: "success" });
      setLibraryVersion((version) => version + 1);
    } catch (error) {
      setSkillAction({
        path: null,
        message: String(error),
        type: "error",
      });
    }
  };

  const installToAgent = (skill: InstalledSkill) => {
    if (!skill.libraryPath) return;
    const firstAvailable = agents.find(
      (agent) => !skill.enabledAgents.includes(agent.id),
    )?.id;
    const target = installTargets[skill.path] ?? firstAvailable;
    if (!target) return;
    const targetName = agents.find((agent) => agent.id === target)?.label;
    if (!window.confirm(`将 ${skill.name} 复制到 ${targetName} 吗？`)) {
      return;
    }
    runSkillAction(
      skill.path,
      () =>
        invoke("copy_library_skill_to_agent", {
          skillPath: skill.libraryPath,
          agent: target,
        }),
      `${skill.name} 已复制到 ${targetName}。`,
    );
  };

  const deleteFromLibrary = (skill: InstalledSkill) => {
    if (!skill.libraryPath) return;
    if (
      !window.confirm(
        `确定从 all_skills 删除 ${skill.name} 吗？其他 Agent 中的副本不会删除。`,
      )
    ) {
      return;
    }
    runSkillAction(
      skill.path,
      () =>
        invoke("delete_library_skill", {
          skillPath: skill.libraryPath,
        }),
      `${skill.name} 已从 all_skills 删除。`,
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
                      const availableAgents = agents.filter(
                        (agent) => !skill.enabledAgents.includes(agent.id),
                      );
                      return (
                      <li
                        key={skill.skillId}
                        className={`skill-item ${
                          isExpanded ? "skill-item--expanded" : ""
                        }`}
                      >
                        <span className="skill-item__mark" />
                        <div className="skill-item__body">
                          <div className="skill-item__title">
                            <p>{skill.name}</p>
                            {skill.isBuiltIn && (
                              <span className="skill-item__badge">内置</span>
                            )}
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
                                      onChange={(event) =>
                                        setInstallTargets((current) => ({
                                          ...current,
                                          [skill.path]: event.currentTarget
                                            .value as AgentId,
                                        }))
                                      }
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
                              {skill.inLibrary &&
                                skill.libraryPath &&
                                skill.enabledAgents.length === 0 && (
                                <button
                                  className="skill-delete-button"
                                  type="button"
                                  disabled={skillAction.path === skill.path}
                                  onClick={() => deleteFromLibrary(skill)}
                                >
                                  删除备份
                                </button>
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
