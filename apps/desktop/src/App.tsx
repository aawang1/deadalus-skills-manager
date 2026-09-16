import {
  type CSSProperties,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { api, isNativeRuntime as detectNativeRuntime } from "./api";
import { filterCanonicalSkills } from "./canonical";
import { CreateCategoryDialog } from "./components/CreateCategoryDialog";
import { SemanticSearch } from "./components/SemanticSearch";
import { SettingsDialog } from "./components/SettingsDialog";
import { SkillGraph } from "./components/SkillGraph";
import type {
  AgentId,
  AgentSkillsResponse,
  CanonicalSnapshot,
  CreateCustomSkillCategoryRequest,
  CustomSkillCategory,
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

type NavigationGroup = "agents" | "custom" | "projects";
type CategoryCopyMode = "incremental" | "overwrite";
type AgentOverwriteRemoval = {
  skill: InstalledSkill;
  uninstallPlan: SkillActionPlan;
  disablePlan: SkillActionPlan;
};
type AgentOverwriteDialogState = {
  sourceName: string;
  targetAgent: AgentId;
  targetLabel: string;
  additions: InstalledSkill[];
  reenable: InstalledSkill[];
  removals: AgentOverwriteRemoval[];
  retainedCount: number;
  busy: boolean;
  error?: string;
};

function App() {
  const isNativeRuntime = detectNativeRuntime();
  const [isKeyDialogOpen, setIsKeyDialogOpen] = useState(false);
  const [isAssistantOpen, setIsAssistantOpen] = useState(false);
  const [isCreateCategoryOpen, setIsCreateCategoryOpen] = useState(false);
  const [activeView, setActiveView] = useState<ViewId | null>(null);
  const [isSkillsListCollapsed, setIsSkillsListCollapsed] = useState(false);
  const [expandedNavigationGroups, setExpandedNavigationGroups] = useState<
    Record<NavigationGroup, boolean>
  >({ agents: true, custom: true, projects: true });
  const [customCategories, setCustomCategories] = useState<CustomSkillCategory[]>([]);
  const [categoryContextMenu, setCategoryContextMenu] = useState<{
    category: CustomSkillCategory;
    x: number;
    y: number;
  }>();
  const [categoryDeleteDialog, setCategoryDeleteDialog] = useState<{
    category: CustomSkillCategory;
    busy: boolean;
    error?: string;
  }>();
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
  const [customCategoryTargets, setCustomCategoryTargets] = useState<
    Record<string, string>
  >({});
  const [categoryCopyTarget, setCategoryCopyTarget] = useState("");
  const [categoryCopyMode, setCategoryCopyMode] =
    useState<CategoryCopyMode>("incremental");
  const [categoryCopyBusy, setCategoryCopyBusy] = useState(false);
  const [agentOverwriteDialog, setAgentOverwriteDialog] =
    useState<AgentOverwriteDialogState>();
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
  const categoryMenuButtonRef = useRef<HTMLButtonElement>(null);
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

  useEffect(() => {
    if (!categoryContextMenu) return;
    requestAnimationFrame(() => categoryMenuButtonRef.current?.focus());
  }, [categoryContextMenu]);

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
    let current = true;
    api
      .listCustomSkillCategories()
      .then((categories) => current && setCustomCategories(categories))
      .catch((error) => {
        if (!current) return;
        setSkillAction({ path: null, message: String(error), type: "error" });
      });
    return () => {
      current = false;
    };
  }, [isNativeRuntime]);

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

  const toggleNavigationGroup = (group: NavigationGroup) => {
    setExpandedNavigationGroups((current) => ({
      ...current,
      [group]: !current[group],
    }));
  };

  const activeCustomCategory = activeView?.startsWith("custom:")
    ? customCategories.find(
        (category) => `custom:${category.categoryId}` === activeView,
      )
    : undefined;
  const activeAgent = agents.find((agent) => agent.id === activeView)?.id;
  const activeCustomSkillIds = useMemo(
    () => new Set(activeCustomCategory?.skillIds ?? []),
    [activeCustomCategory],
  );
  const activeCategorySkills = useMemo(
    () =>
      (skillResult?.skills ?? []).filter(
        (skill) =>
          activeCustomSkillIds.has(skill.skillId) && !skill.isBuiltIn,
      ),
    [activeCustomSkillIds, skillResult],
  );
  const categoryCopyOptions = useMemo(() => {
    if (!activeCustomCategory) return [];
    return [
      ...agents.map((agent) => ({
        value: `agent:${agent.id}`,
        label: agent.label,
      })),
      ...customCategories
        .filter((category) => category.categoryId !== activeCustomCategory.categoryId)
        .map((category) => ({
          value: `custom:${category.categoryId}`,
          label: `类别 · ${category.name}`,
        })),
    ];
  }, [activeCustomCategory, customCategories]);
  const effectiveCategoryCopyTarget = categoryCopyOptions.some(
    (option) => option.value === categoryCopyTarget,
  )
    ? categoryCopyTarget
    : categoryCopyOptions[0]?.value ?? "";
  const effectiveCategoryCopyMode = categoryCopyMode;
  const agentOverwriteRiskRemovals =
    agentOverwriteDialog?.removals.filter(
      (removal) =>
        removal.uninstallPlan.sharedInstallation ||
        !removal.uninstallPlan.allowed,
    ) ?? [];
  const canForceAgentOverwrite =
    agentOverwriteDialog?.removals.every(
      (removal) => removal.uninstallPlan.allowed,
    ) ?? false;
  const canSafelyAgentOverwrite =
    agentOverwriteDialog?.removals.every((removal) => {
      const useDisable =
        removal.uninstallPlan.sharedInstallation ||
        !removal.uninstallPlan.allowed;
      return useDisable
        ? removal.disablePlan.allowed
        : removal.uninstallPlan.allowed;
    }) ?? false;

  const createCustomCategory = async (
    request: CreateCustomSkillCategoryRequest,
  ) => {
    const category = await api.createCustomSkillCategory(request);
    setCustomCategories((current) =>
      [...current, category].sort((left, right) =>
        left.name.localeCompare(right.name),
      ),
    );
    setExpandedNavigationGroups((current) => ({ ...current, custom: true }));
    setIsCreateCategoryOpen(false);
    toggleView(`custom:${category.categoryId}`);
  };

  const requestDeleteCustomCategory = (category: CustomSkillCategory) => {
    setCategoryContextMenu(undefined);
    setCategoryDeleteDialog({ category, busy: false });
  };

  const deleteCustomCategory = async () => {
    if (!categoryDeleteDialog || categoryDeleteDialog.busy) return;
    const category = categoryDeleteDialog.category;
    setCategoryDeleteDialog({ category, busy: true });
    try {
      const deleted = await api.deleteCustomSkillCategory(category.categoryId);
      if (!deleted) throw new Error("该自定义类别已不存在，请刷新后重试。");
      setCustomCategories((current) =>
        current.filter((item) => item.categoryId !== category.categoryId),
      );
      if (activeView === `custom:${category.categoryId}`) {
        setActiveView(null);
        setIsSkillsListCollapsed(false);
        setSearchQuery("");
      }
      setCategoryDeleteDialog(undefined);
    } catch (error) {
      setCategoryDeleteDialog({
        category,
        busy: false,
        error: String(error),
      });
    }
  };

  const updateCustomCategoryState = (category: CustomSkillCategory) => {
    setCustomCategories((current) =>
      current.map((item) =>
        item.categoryId === category.categoryId ? category : item,
      ),
    );
  };

  const copySkillToCustomCategory = async (
    skill: InstalledSkill,
    categoryId: string,
  ) => {
    if (!categoryId || skill.isBuiltIn) return;
    const target = customCategories.find(
      (category) => category.categoryId === categoryId,
    );
    if (!target || target.skillIds.includes(skill.skillId)) return;
    setSkillAction({ path: skill.path, message: "", type: "idle" });
    try {
      const result = await api.addSkillsToCustomCategory(categoryId, [skill.skillId]);
      updateCustomCategoryState(result.category);
      setSkillAction({
        path: null,
        message:
          result.addedCount > 0
            ? `${skill.name} 已加入 ${target.name}。`
            : `${target.name} 已包含该 Skill。`,
        type: "success",
      });
    } catch (error) {
      setSkillAction({ path: null, message: String(error), type: "error" });
    }
  };

  const copyActiveCategory = async () => {
    if (
      !activeCustomCategory ||
      !effectiveCategoryCopyTarget ||
      categoryCopyBusy
    ) {
      return;
    }
    const [targetType, targetId] = effectiveCategoryCopyTarget.split(":", 2);
    const targetLabel =
      categoryCopyOptions.find(
        (option) => option.value === effectiveCategoryCopyTarget,
      )?.label ?? "目标";
    if (targetType === "agent" && effectiveCategoryCopyMode === "overwrite") {
      const targetAgent = targetId as AgentId;
      const sourceIds = new Set(activeCategorySkills.map((skill) => skill.skillId));
      const targetSkills = (skillResult?.skills ?? []).filter(
        (skill) =>
          !skill.isBuiltIn && skill.enabledAgents.includes(targetAgent),
      );
      const additions = activeCategorySkills.filter(
        (skill) => !skill.enabledAgents.includes(targetAgent),
      );
      const reenable = activeCategorySkills.filter(
        (skill) =>
          skill.enabledAgents.includes(targetAgent) &&
          skill.disabledAgents.includes(targetAgent),
      );
      const removalSkills = targetSkills.filter(
        (skill) => !sourceIds.has(skill.skillId),
      );
      setCategoryCopyBusy(true);
      setSkillAction({ path: null, message: "", type: "idle" });
      try {
        const removals = await Promise.all(
          removalSkills.map(async (skill) => {
            const [uninstallPlan, disablePlan] = await Promise.all([
              api.prepareSkillAction(skill.skillId, targetAgent, "uninstall"),
              api.prepareSkillAction(skill.skillId, targetAgent, "disable"),
            ]);
            return { skill, uninstallPlan, disablePlan };
          }),
        );
        setAgentOverwriteDialog({
          sourceName: activeCustomCategory.name,
          targetAgent,
          targetLabel,
          additions,
          reenable,
          removals,
          retainedCount:
            activeCategorySkills.length - additions.length - reenable.length,
          busy: false,
        });
      } catch (error) {
        setSkillAction({
          path: null,
          message: `无法核对 Agent 覆盖范围：${String(error)}`,
          type: "error",
        });
      } finally {
        setCategoryCopyBusy(false);
      }
      return;
    }
    const targetCategory =
      targetType === "custom"
        ? customCategories.find((category) => category.categoryId === targetId)
        : undefined;
    if (targetType === "custom" && !targetCategory) {
      setSkillAction({
        path: null,
        message: "目标虚拟类别已不存在。",
        type: "error",
      });
      return;
    }
    const sourceIds = new Set(activeCustomCategory.skillIds);
    const targetIds = new Set(targetCategory?.skillIds ?? []);
    const overwriteAdded = [...sourceIds].filter((id) => !targetIds.has(id)).length;
    const overwriteRemoved = [...targetIds].filter((id) => !sourceIds.has(id)).length;
    const confirmation =
      effectiveCategoryCopyMode === "overwrite"
        ? `用 ${activeCustomCategory.name} 覆盖 ${targetLabel} 吗？将新增 ${overwriteAdded} 个、移除 ${overwriteRemoved} 个成员。此操作只修改类别归属，不会删除 Skill 文件。`
        : `将 ${activeCustomCategory.name} 中目标尚未包含的 Skills 增量复制到 ${targetLabel} 吗？`;
    if (!window.confirm(confirmation)) {
      return;
    }
    setCategoryCopyBusy(true);
    setSkillAction({ path: null, message: "", type: "idle" });
    try {
      if (targetType === "custom") {
        if (!targetCategory) throw new Error("目标虚拟类别已不存在。");
        if (effectiveCategoryCopyMode === "overwrite") {
          const result = await api.replaceCustomCategoryMembers(
            activeCustomCategory.categoryId,
            targetId,
          );
          updateCustomCategoryState(result.category);
          setSkillAction({
            path: null,
            message: `已覆盖 ${targetLabel}：新增 ${result.addedCount} 个，移除 ${result.removedCount} 个，保留 ${result.unchangedCount} 个。`,
            type: "success",
          });
          return;
        }
        const missing = activeCategorySkills.filter(
          (skill) => !targetCategory.skillIds.includes(skill.skillId),
        );
        if (missing.length === 0) {
          setSkillAction({
            path: null,
            message: `${targetLabel} 已包含该类别的全部 Skills。`,
            type: "success",
          });
          return;
        }
        const result = await api.addSkillsToCustomCategory(
          targetId,
          missing.map((skill) => skill.skillId),
        );
        updateCustomCategoryState(result.category);
        const alreadyPresent = activeCategorySkills.length - missing.length;
        setSkillAction({
          path: null,
          message: `已向 ${targetLabel} 加入 ${result.addedCount} 个，跳过 ${alreadyPresent + result.skippedCount} 个已有或不可加入的 Skills。`,
          type: "success",
        });
      } else if (targetType === "agent") {
        const targetAgent = targetId as AgentId;
        const missing = activeCategorySkills.filter(
          (skill) => !skill.enabledAgents.includes(targetAgent),
        );
        let copied = 0;
        let failed = 0;
        for (const skill of missing) {
          if (!skill.libraryPath || !skill.inLibrary) {
            failed += 1;
            continue;
          }
          try {
            await api.copyLibrarySkillToAgent(skill.libraryPath, targetAgent);
            copied += 1;
          } catch {
            failed += 1;
          }
        }
        const skipped = activeCategorySkills.length - missing.length;
        setSkillAction({
          path: null,
          message: `已复制 ${copied} 个到 ${targetLabel}，跳过 ${skipped} 个已有 Skills${failed ? `，${failed} 个复制失败` : ""}。`,
          type: failed ? "error" : "success",
        });
        if (copied > 0) setLibraryVersion((version) => version + 1);
      }
    } catch (error) {
      setSkillAction({ path: null, message: String(error), type: "error" });
    } finally {
      setCategoryCopyBusy(false);
    }
  };

  const executeAgentOverwrite = async (
    sharedStrategy: "uninstall" | "disable",
  ) => {
    const dialog = agentOverwriteDialog;
    if (!dialog || dialog.busy) return;
    setAgentOverwriteDialog({ ...dialog, busy: true, error: undefined });
    let copied = 0;
    let enabled = 0;
    let removed = 0;
    let disabled = 0;
    try {
      for (const skill of dialog.additions) {
        if (!skill.libraryPath || !skill.inLibrary) {
          throw new Error(`${skill.name} 没有可复制的 All Skills 后备路径。`);
        }
        await api.copyLibrarySkillToAgent(skill.libraryPath, dialog.targetAgent);
        copied += 1;
      }
      for (const skill of dialog.reenable) {
        await api.enableSkillForAgent(skill.skillId, dialog.targetAgent);
        enabled += 1;
      }
      for (const removal of dialog.removals) {
        const useDisable =
          sharedStrategy === "disable" &&
          (removal.uninstallPlan.sharedInstallation ||
            !removal.uninstallPlan.allowed);
        const plan = useDisable ? removal.disablePlan : removal.uninstallPlan;
        if (!plan.allowed) {
          throw new Error(
            `${removal.skill.name} 无法${useDisable ? "禁用" : "卸载"}：${plan.reason ?? "未通过操作预检"}`,
          );
        }
        if (useDisable) {
          await api.disableSkillForAgent(
            removal.skill.skillId,
            dialog.targetAgent,
          );
          disabled += 1;
        } else {
          await api.uninstallSkill(removal.skill.skillId, dialog.targetAgent);
          removed += 1;
        }
      }
      setSkillAction({
        path: null,
        message: `已覆盖 ${dialog.targetLabel}：新增 ${copied} 个，解禁 ${enabled} 个，卸载 ${removed} 个${disabled ? `，禁用共享项 ${disabled} 个` : ""}；内置 Skills 保持不变。`,
        type: "success",
      });
      setAgentOverwriteDialog(undefined);
      setLibraryVersion((version) => version + 1);
    } catch (error) {
      setAgentOverwriteDialog({
        ...dialog,
        busy: false,
        error: `覆盖中断：${String(error)} 已完成新增 ${copied}、解禁 ${enabled}、卸载 ${removed}、禁用 ${disabled}。`,
      });
    }
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
      customSkillIds: activeCustomSkillIds,
    });
  }, [activeCustomSkillIds, activeView, agentFilter, searchQuery, skillResult]);

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

        <nav className="agent-actions" aria-label="Skill 类别">
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

          <NavigationGroupToggle
            id="agents"
            label="Agent 全局"
            shortLabel="Agents"
            expanded={expandedNavigationGroups.agents}
            onToggle={() => toggleNavigationGroup("agents")}
          >
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
          </NavigationGroupToggle>

          <NavigationGroupToggle
            id="custom"
            label="自定义类别"
            shortLabel="自定义"
            expanded={expandedNavigationGroups.custom}
            onToggle={() => toggleNavigationGroup("custom")}
          >
            {customCategories.length === 0 ? (
              <div className="navigation-group__empty">暂无自定义类别</div>
            ) : (
              customCategories.map((category) => {
                const viewId = `custom:${category.categoryId}` as const;
                return (
                  <button
                    key={category.categoryId}
                    className={`agent-button category-button ${
                      activeView === viewId ? "agent-button--active" : ""
                    }`}
                    type="button"
                    aria-label={`${category.name} Skills`}
                    aria-pressed={activeView === viewId}
                    title={category.description}
                    onClick={() => toggleView(viewId)}
                    onContextMenu={(event) => {
                      event.preventDefault();
                      setCategoryContextMenu({
                        category,
                        x: Math.min(event.clientX, window.innerWidth - 176),
                        y: Math.min(event.clientY, window.innerHeight - 64),
                      });
                    }}
                  >
                    <span
                      className="category-button__dot"
                      style={{ backgroundColor: category.color }}
                      aria-hidden="true"
                    />
                    <span className="agent-button__full">{category.name}</span>
                  </button>
                );
              })
            )}
            <button
              className="agent-button agent-button--add"
              type="button"
              aria-label="新建自定义类别"
              title="新建自定义类别"
              onClick={() => setIsCreateCategoryOpen(true)}
            >
              <span aria-hidden="true">＋</span>
              <span className="agent-button__full">新建类别</span>
            </button>
          </NavigationGroupToggle>

          <NavigationGroupToggle
            id="projects"
            label="项目"
            shortLabel="项目"
            expanded={expandedNavigationGroups.projects}
            onToggle={() => toggleNavigationGroup("projects")}
          >
            <div className="navigation-group__empty">暂无项目</div>
          </NavigationGroupToggle>
        </nav>

        <p className="page-code">A1</p>
      </aside>

      {categoryContextMenu && (
        <div
          className="category-context-layer"
          role="presentation"
          onMouseDown={() => setCategoryContextMenu(undefined)}
          onContextMenu={(event) => {
            event.preventDefault();
            setCategoryContextMenu(undefined);
          }}
        >
          <div
            className="category-context-menu"
            role="menu"
            aria-label={`${categoryContextMenu.category.name} 类别操作`}
            style={{ left: categoryContextMenu.x, top: categoryContextMenu.y }}
            onMouseDown={(event) => event.stopPropagation()}
          >
            <button
              ref={categoryMenuButtonRef}
              type="button"
              role="menuitem"
              onClick={() => requestDeleteCustomCategory(categoryContextMenu.category)}
            >
              <span aria-hidden="true">×</span>
              删除该类别
            </button>
          </div>
        </div>
      )}

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
                    {activeView === "all"
                      ? "SKILL LIBRARY"
                      : activeCustomCategory
                        ? "CUSTOM CATEGORY"
                        : "AGENT SKILLS"}
                  </p>
                  <h2>
                    {activeView === "all"
                      ? "所有 Skills"
                      : activeCustomCategory?.name ??
                        agents.find((agent) => agent.id === activeView)?.label}
                  </h2>
                  {activeCustomCategory && (
                    <p className="custom-category-description">
                      {activeCustomCategory.description}
                    </p>
                  )}
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

            {!isSkillsListCollapsed && activeCustomCategory && (
              <div className="custom-category-copybar">
                <label>
                  <span>复制类别到</span>
                  <select
                    aria-label={`选择 ${activeCustomCategory.name} 的复制目标`}
                    value={effectiveCategoryCopyTarget}
                    disabled={categoryCopyBusy || categoryCopyOptions.length === 0}
                    onChange={(event) =>
                      setCategoryCopyTarget(event.currentTarget.value)
                    }
                  >
                    {categoryCopyOptions.map((option) => (
                      <option key={option.value} value={option.value}>
                        {option.label}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  <span>复制方式</span>
                  <select
                    aria-label={`选择 ${activeCustomCategory.name} 的复制方式`}
                    value={effectiveCategoryCopyMode}
                    disabled={categoryCopyBusy}
                    title={
                      "增量复制只补齐缺少项；覆盖复制会同步新增和移除项"
                    }
                    onChange={(event) =>
                      setCategoryCopyMode(
                        event.currentTarget.value as CategoryCopyMode,
                      )
                    }
                  >
                    <option value="incremental">增量复制</option>
                    <option value="overwrite">覆盖复制</option>
                  </select>
                </label>
                <button
                  type="button"
                  disabled={
                    categoryCopyBusy ||
                    (effectiveCategoryCopyMode === "incremental" &&
                      activeCategorySkills.length === 0) ||
                    !effectiveCategoryCopyTarget
                  }
                  onClick={copyActiveCategory}
                >
                  {categoryCopyBusy
                    ? effectiveCategoryCopyMode === "overwrite" &&
                      effectiveCategoryCopyTarget.startsWith("agent:")
                      ? "正在核对…"
                      : "正在复制…"
                    : effectiveCategoryCopyMode === "overwrite"
                      ? effectiveCategoryCopyTarget.startsWith("agent:")
                        ? "覆盖目标 Agent"
                        : "覆盖目标类别"
                      : "增量复制"}
                </button>
              </div>
            )}

            {!isSkillsListCollapsed && (
              <>
            <div className="skills-content">
              {(activeView === "all" || activeCustomCategory) && (
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
                  {activeView === "all" && <div className="agent-filters" aria-label="按 Agent 筛选">
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
                  </div>}
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
                        : activeCustomCategory
                          ? "该自定义类别中没有匹配项"
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
                      const isDisabled = Boolean(
                        activeAgent && skill.disabledAgents.includes(activeAgent),
                      );
                      const availableAgents = agents.filter(
                        (agent) => !skill.enabledAgents.includes(agent.id),
                      );
                      const availableCustomCategories = skill.isBuiltIn
                        ? []
                        : customCategories.filter(
                            (category) => !category.skillIds.includes(skill.skillId),
                          );
                      const selectedCustomCategoryId = availableCustomCategories.some(
                        (category) =>
                          category.categoryId === customCategoryTargets[skill.path],
                      )
                        ? customCategoryTargets[skill.path]
                        : availableCustomCategories[0]?.categoryId ?? "";
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
                            {!activeCustomCategory && <div className="skill-item__quick-actions">
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
                            </div>}
                          </div>
                          {skill.description && <span>{skill.description}</span>}
                          {activeAgent && (
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
                              <div className="skill-install-control skill-install-control--category">
                                <select
                                  aria-label={`选择 ${skill.name} 的虚拟类别目标`}
                                  value={selectedCustomCategoryId}
                                  disabled={availableCustomCategories.length === 0}
                                  onChange={(event) => {
                                    const categoryId = event.currentTarget.value;
                                    setCustomCategoryTargets((current) => ({
                                      ...current,
                                      [skill.path]: categoryId,
                                    }));
                                  }}
                                >
                                  {availableCustomCategories.length === 0 ? (
                                    <option value="">
                                      {skill.isBuiltIn
                                        ? "内置 Skills 不加入虚拟类别"
                                        : "所有类别均已包含"}
                                    </option>
                                  ) : (
                                    availableCustomCategories.map((category) => (
                                      <option
                                        key={category.categoryId}
                                        value={category.categoryId}
                                      >
                                        {category.name}
                                      </option>
                                    ))
                                  )}
                                </select>
                                <button
                                  type="button"
                                  disabled={
                                    skillAction.path === skill.path ||
                                    !selectedCustomCategoryId
                                  }
                                  onClick={() =>
                                    copySkillToCustomCategory(
                                      skill,
                                      selectedCustomCategoryId,
                                    )
                                  }
                                >
                                  复制到类别
                                </button>
                              </div>
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

      {agentOverwriteDialog && (
        <div
          className="skill-action-backdrop"
          role="presentation"
          onMouseDown={() =>
            !agentOverwriteDialog.busy && setAgentOverwriteDialog(undefined)
          }
        >
          <section
            className="skill-action-dialog agent-overwrite-dialog"
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="agent-overwrite-title"
            onMouseDown={(event) => event.stopPropagation()}
          >
            <header>
              <div>
                <p className="eyebrow">AGENT OVERWRITE</p>
                <h3 id="agent-overwrite-title">
                  覆盖 {agentOverwriteDialog.targetLabel}
                </h3>
              </div>
              <button
                type="button"
                className="modal-close"
                aria-label="关闭 Agent 覆盖确认"
                disabled={agentOverwriteDialog.busy}
                onClick={() => setAgentOverwriteDialog(undefined)}
              >
                ×
              </button>
            </header>
            <strong>{agentOverwriteDialog.sourceName}</strong>
            <p>
              将目标 Agent 的普通 Skills 同步为该虚拟类别。内置 Skills
              不参与覆盖并保持原样。
            </p>
            <div className="agent-overwrite-dialog__counts" aria-label="覆盖变更摘要">
              <span>新增 {agentOverwriteDialog.additions.length}</span>
              <span>解禁 {agentOverwriteDialog.reenable.length}</span>
              <span>移除 {agentOverwriteDialog.removals.length}</span>
              <span>保留 {agentOverwriteDialog.retainedCount}</span>
            </div>
            {agentOverwriteRiskRemovals.length > 0 && (
              <div className="skill-action-dialog__impact">
                <strong>
                  {agentOverwriteRiskRemovals.length} 个待移除 Skill
                  会影响其他 Agent 或不能直接卸载。
                </strong>
                <p>
                  选择“禁用风险项”只在当前 Agent 中禁用这些 Skill；选择“仍然删除”会按实际共享路径卸载，可能同时影响其他 Agent。
                </p>
                <details>
                  <summary>查看影响范围</summary>
                  {agentOverwriteRiskRemovals.map((removal) => (
                    <div
                      className="agent-overwrite-dialog__risk"
                      key={removal.skill.skillId}
                    >
                      <span>{removal.skill.name}</span>
                      <small>
                        {removal.uninstallPlan.sharedInstallation
                          ? `共享：${removal.uninstallPlan.affectedAgents.join("、")}`
                          : removal.uninstallPlan.reason ?? "需要改为禁用"}
                      </small>
                    </div>
                  ))}
                </details>
              </div>
            )}
            {agentOverwriteDialog.error && (
              <p className="skill-action-dialog__error" role="alert">
                {agentOverwriteDialog.error}
              </p>
            )}
            {agentOverwriteDialog.busy && (
              <div className="skill-action-dialog__state">
                <span className="spinner" />正在同步 Agent Skills…
              </div>
            )}
            <footer>
              <button
                type="button"
                disabled={agentOverwriteDialog.busy}
                onClick={() => setAgentOverwriteDialog(undefined)}
              >
                取消
              </button>
              {agentOverwriteRiskRemovals.length > 0 && (
                <button
                  type="button"
                  className="warning-button"
                  disabled={
                    agentOverwriteDialog.busy || !canSafelyAgentOverwrite
                  }
                  title={
                    canSafelyAgentOverwrite
                      ? "独立项卸载，共享或不可卸载项仅禁用当前 Agent"
                      : "至少一个风险项无法只在当前 Agent 中禁用"
                  }
                  onClick={() => executeAgentOverwrite("disable")}
                >
                  禁用风险项
                </button>
              )}
              <button
                type="button"
                className="danger-button"
                disabled={
                  agentOverwriteDialog.busy || !canForceAgentOverwrite
                }
                title={
                  canForceAgentOverwrite
                    ? "执行新增、解禁与物理卸载"
                    : "至少一个 Skill 未通过卸载预检"
                }
                onClick={() => executeAgentOverwrite("uninstall")}
              >
                {agentOverwriteRiskRemovals.length > 0
                  ? "仍然删除"
                  : "确认覆盖"}
              </button>
            </footer>
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

      {categoryDeleteDialog && (
        <div className="skill-action-backdrop" role="presentation">
          <section
            className="skill-action-dialog category-delete-dialog"
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="category-delete-title"
          >
            <header>
              <div>
                <p className="eyebrow">CUSTOM CATEGORY</p>
                <h3 id="category-delete-title">删除自定义类别</h3>
              </div>
              <button
                type="button"
                className="modal-close"
                aria-label="关闭删除类别确认"
                disabled={categoryDeleteDialog.busy}
                onClick={() => setCategoryDeleteDialog(undefined)}
              >
                ×
              </button>
            </header>
            <strong>{categoryDeleteDialog.category.name}</strong>
            <p>确认删除这个虚拟类别？此操作只删除类别及成员归属，不会删除任何 Skill、向量数据或 Agent 安装。</p>
            {categoryDeleteDialog.error && (
              <p className="skill-action-dialog__error" role="alert">
                {categoryDeleteDialog.error}
              </p>
            )}
            <footer>
              <button
                type="button"
                disabled={categoryDeleteDialog.busy}
                onClick={() => setCategoryDeleteDialog(undefined)}
              >
                取消
              </button>
              <button
                type="button"
                className="danger-button"
                disabled={categoryDeleteDialog.busy}
                onClick={deleteCustomCategory}
              >
                {categoryDeleteDialog.busy ? "正在删除…" : "确认删除"}
              </button>
            </footer>
          </section>
        </div>
      )}

      <main className="main-platform" aria-label="A1 可视化工作区">
            {activeView && (
          <SkillGraph
            activeView={activeView}
            viewLabel={activeCustomCategory?.name}
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
      {isCreateCategoryOpen && (
        <CreateCategoryDialog
          skills={skillResult?.skills ?? []}
          onClose={() => setIsCreateCategoryOpen(false)}
          onCreate={createCustomCategory}
        />
      )}
    </div>
  );
}

function NavigationGroupToggle({
  id,
  label,
  shortLabel,
  expanded,
  onToggle,
  children,
}: {
  id: NavigationGroup;
  label: string;
  shortLabel: string;
  expanded: boolean;
  onToggle: () => void;
  children: ReactNode;
}) {
  const contentId = `navigation-group-${id}`;
  return (
    <section className="navigation-group">
      <button
        className="navigation-group__toggle"
        type="button"
        aria-label={label}
        aria-expanded={expanded}
        aria-controls={contentId}
        onClick={onToggle}
      >
        <span className="navigation-group__label">{label}</span>
        <span className="navigation-group__short">{shortLabel}</span>
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path
            d={expanded ? "M4 6l4 4 4-4" : "M6 4l4 4-4 4"}
            fill="none"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </button>
      {expanded && (
        <div id={contentId} className="navigation-group__content">
          {children}
        </div>
      )}
    </section>
  );
}

export default App;
