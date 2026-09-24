import { tr, getLanguage, useLanguage } from "./i18n";
import { DisplaySummary } from "./components/DisplaySummary";
import { type CSSProperties, type ReactNode, useCallback, useEffect, useMemo, useRef, useState, } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, isNativeRuntime as detectNativeRuntime } from "./api";
import { filterCanonicalSkills } from "./canonical";
import { CreateCategoryDialog } from "./components/CreateCategoryDialog";
import { AgentComposer } from "./components/AgentComposer";
import { SettingsDialog } from "./components/SettingsDialog";
import { SkillGraph } from "./components/SkillGraph";
import type { AgentId, AgentSkillsResponse, CanonicalSnapshot, CreateCustomSkillCategoryRequest, CustomSkillCategory, InstalledSkill, ProgressEvent, SkillActionPlan, ViewId, } from "./types";
import "./App.css";
const agents: Array<{
    id: AgentId;
    label: string;
    shortLabel: string;
}> = [
    { id: "claude-code", label: "Claude Code", shortLabel: "Claude" },
    { id: "cursor", label: "Cursor", shortLabel: "Cursor" },
    { id: "codex", label: "Codex", shortLabel: "Codex" },
];
type NavigationGroup = "agents" | "custom" | "projects";
type CategoryCopyMode = "incremental" | "overwrite";
const AGENT_WINDOWS_STORAGE_KEY = "deadalus.agent-windows.v3";
interface AgentWindowRegistry {
    ids: number[];
    nextId: number;
    bindings: Record<string, ViewId>;
}
function isStoredViewId(value: unknown): value is ViewId {
    return value === "all" || agents.some((agent) => agent.id === value) ||
        (typeof value === "string" && value.startsWith("custom:") && value.length > 7);
}
function savedAgentWindows(): AgentWindowRegistry {
    try {
        const saved = window.localStorage.getItem(AGENT_WINDOWS_STORAGE_KEY);
        if (saved) {
            const registry: AgentWindowRegistry = JSON.parse(saved);
            const ids = registry?.ids;
            const bindings = registry?.bindings;
            if (Array.isArray(ids) &&
                ids.length <= 100 &&
                ids.every((id) => Number.isSafeInteger(id) && id > 0) &&
                new Set(ids).size === ids.length &&
                Number.isSafeInteger(registry.nextId) &&
                registry.nextId > Math.max(0, ...ids) &&
                bindings &&
                typeof bindings === "object" &&
                ids.every((id) => isStoredViewId(bindings[String(id)]))) {
                return { ids, nextId: registry.nextId, bindings };
            }
        }
    }
    catch {
        // A malformed or unavailable local record falls back to no bound windows.
    }
    return { ids: [], nextId: 1, bindings: {} };
}
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
    useLanguage();
    const isNativeRuntime = detectNativeRuntime();
    const [isKeyDialogOpen, setIsKeyDialogOpen] = useState(false);
    const [agentWindows, setAgentWindows] = useState(savedAgentWindows);
    const [activeAgentWindow, setActiveAgentWindow] = useState<number | null>(null);
    const [agentHighlights, setAgentHighlights] = useState<Record<number, {
        viewId: ViewId;
        skillIds: string[];
    }>>({});
    const isAssistantOpen = activeAgentWindow !== null;
    const [agentWindowContextMenu, setAgentWindowContextMenu] = useState<{
        windowId: number;
        x: number;
        y: number;
    }>();
    const [agentWindowDeleteDialog, setAgentWindowDeleteDialog] = useState<number>();
    const [isCreateCategoryOpen, setIsCreateCategoryOpen] = useState(false);
    const [isCreateProjectOpen, setIsCreateProjectOpen] = useState(false);
    const [activeView, setActiveView] = useState<ViewId | null>(null);
    const [isSkillsListCollapsed, setIsSkillsListCollapsed] = useState(false);
    const [expandedNavigationGroups, setExpandedNavigationGroups] = useState<Record<NavigationGroup, boolean>>({ agents: true, custom: true, projects: true });
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
    const [skillResult, setSkillResult] = useState<AgentSkillsResponse | null>(null);
    const [skillState, setSkillState] = useState<"idle" | "loading" | "ready" | "error">("idle");
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
    const [installTargets, setInstallTargets] = useState<Record<string, AgentId>>({});
    const [customCategoryTargets, setCustomCategoryTargets] = useState<Record<string, string>>({});
    const [categoryCopyTarget, setCategoryCopyTarget] = useState("");
    const [categoryCopyMode, setCategoryCopyMode] = useState<CategoryCopyMode>("incremental");
    const [categoryCopyBusy, setCategoryCopyBusy] = useState(false);
    const [agentOverwriteDialog, setAgentOverwriteDialog] = useState<AgentOverwriteDialogState>();
    const [expandedSkills, setExpandedSkills] = useState<Set<string>>(() => new Set());
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
    const agentWindowMenuButtonRef = useRef<HTMLButtonElement>(null);
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
        if (!isKeyDialogOpen)
            return;
        requestAnimationFrame(() => keyInputRef.current?.focus());
    }, [isKeyDialogOpen]);
    useEffect(() => {
        try {
            window.localStorage.setItem(AGENT_WINDOWS_STORAGE_KEY, JSON.stringify(agentWindows));
        }
        catch {
            // The in-memory windows remain usable if local storage is unavailable.
        }
    }, [agentWindows]);
    const createAgentWindow = () => {
        if (!activeView || agentWindows.ids.length >= 100)
            return;
        const next = agentWindows.nextId;
        setAgentWindows({
            ids: [...agentWindows.ids, next],
            nextId: next + 1,
            bindings: { ...agentWindows.bindings, [String(next)]: activeView },
        });
        setActiveAgentWindow(next);
    };
    const deleteAgentWindow = () => {
        const windowId = agentWindowDeleteDialog;
        if (windowId === undefined)
            return;
        setAgentWindows((current) => {
            const bindings = { ...current.bindings };
            delete bindings[String(windowId)];
            return {
                ...current,
                ids: current.ids.filter((id) => id !== windowId),
                bindings,
            };
        });
        if (activeAgentWindow === windowId)
            setActiveAgentWindow(null);
        setAgentHighlights((current) => { const next = { ...current }; delete next[windowId]; return next; });
        setAgentWindowDeleteDialog(undefined);
    };
    useEffect(() => {
        if (!categoryContextMenu)
            return;
        requestAnimationFrame(() => categoryMenuButtonRef.current?.focus());
    }, [categoryContextMenu]);
    useEffect(() => {
        if (!agentWindowContextMenu)
            return;
        requestAnimationFrame(() => agentWindowMenuButtonRef.current?.focus());
    }, [agentWindowContextMenu]);
    const loadSkills = useCallback(async (refresh: boolean) => {
        const snapshot: CanonicalSnapshot = await (refresh
            ? api.refreshCanonicalSkillsSnapshot()
            : api.getCanonicalSkillsSnapshot());
        if (refresh) {
            const categories = await api.listCustomSkillCategories();
            setCustomCategories(categories);
        }
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
                setSkillError(tr("浏览器预览无法读取本机目录，请在 Tauri 桌面窗口中使用此功能。"));
                setSkillState("error");
            }
            return;
        }
        let isCurrent = true;
        setSkillState("loading");
        setSkillError("");
        const refreshing = libraryVersion > 0;
        if (refreshing)
            skillRefreshInFlight.current = true;
        loadSkills(refreshing)
            .then((result) => {
            if (!isCurrent)
                return;
            setSkillResult(result);
            setSkillState("ready");
        })
            .catch((error) => {
            if (!isCurrent)
                return;
            setSkillError(String(error));
            setSkillState("error");
        })
            .finally(() => {
            if (!refreshing)
                return;
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
        if (!isNativeRuntime)
            return;
        let current = true;
        api
            .listCustomSkillCategories()
            .then((categories) => current && setCustomCategories(categories))
            .catch((error) => {
            if (!current)
                return;
            setSkillAction({ path: null, message: String(error), type: "error" });
        });
        return () => {
            current = false;
        };
    }, [isNativeRuntime]);
    useEffect(() => {
        if (!isNativeRuntime)
            return;
        let disposed = false;
        let stopListening: (() => void) | undefined;
        let refreshTimer: number | undefined;
        listen("all-skills-changed", () => {
            if (skillRefreshInFlight.current ||
                Date.now() < suppressSkillEventsUntil.current) {
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
            }
            else {
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
        if (!isNativeRuntime)
            return;
        let disposed = false;
        let stopListening: (() => void) | undefined;
        listen<ProgressEvent>("embedding-job-progress", ({ payload }) => {
            if (payload.total > 0 && payload.completed >= payload.total) {
                setGraphRevision((revision) => revision + 1);
            }
        }).then((unlisten) => {
            if (disposed)
                unlisten();
            else
                stopListening = unlisten;
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
        const nextView = closing ? null : view;
        setActiveView(nextView);
        if (activeAgentWindow !== null &&
            agentWindows.bindings[String(activeAgentWindow)] !== nextView) {
            setActiveAgentWindow(null);
        }
        if (closing || openingFromIdle) {
            setIsSkillsListCollapsed(false);
        }
        setSearchQuery("");
        setAgentFilter("all");
        setSkillAction({ path: null, message: "", type: "idle" });
    };
    const toggleAgentWindow = (windowId: number) => {
        const boundView = agentWindows.bindings[String(windowId)];
        if (!boundView)
            return;
        const selected = activeAgentWindow === windowId;
        setActiveView(boundView);
        setIsSkillsListCollapsed(false);
        setSearchQuery("");
        setAgentFilter("all");
        setSkillAction({ path: null, message: "", type: "idle" });
        setActiveAgentWindow(selected ? null : windowId);
    };
    const toggleNavigationGroup = (group: NavigationGroup) => {
        setExpandedNavigationGroups((current) => ({
            ...current,
            [group]: !current[group],
        }));
    };
    const activeCustomCategory = activeView?.startsWith("custom:")
        ? customCategories.find((category) => `custom:${category.categoryId}` === activeView)
        : undefined;
    const activeAgent = agents.find((agent) => agent.id === activeView)?.id;
    const viewLabel = (view: ViewId) => {
        if (view === "all")
            return tr("所有 Skills");
        const agent = agents.find((item) => item.id === view);
        if (agent)
            return agent.label;
        return customCategories.find((category) => `custom:${category.categoryId}` === view)?.name ?? tr("已删除类别");
    };
    const activeCustomSkillIds = useMemo(() => new Set(activeCustomCategory?.skillIds ?? []), [activeCustomCategory]);
    const activeCategorySkills = useMemo(() => (skillResult?.skills ?? []).filter((skill) => activeCustomSkillIds.has(skill.skillId) && !skill.isBuiltIn), [activeCustomSkillIds, skillResult]);
    const categoryCopyOptions = useMemo(() => {
        if (!activeCustomCategory)
            return [];
        return [
            ...agents.map((agent) => ({
                value: `agent:${agent.id}`,
                label: agent.label,
            })),
            ...customCategories
                .filter((category) => category.categoryId !== activeCustomCategory.categoryId)
                .map((category) => ({
                value: `custom:${category.categoryId}`,
                label: tr("{0} · {1}", category.projectRoot ? tr("项目") : tr("类别"), category.name),
            })),
        ];
    }, [activeCustomCategory, customCategories, getLanguage()]);
    const effectiveCategoryCopyTarget = categoryCopyOptions.some((option) => option.value === categoryCopyTarget)
        ? categoryCopyTarget
        : categoryCopyOptions[0]?.value ?? "";
    const effectiveCategoryCopyMode = categoryCopyMode;
    const agentOverwriteRiskRemovals = agentOverwriteDialog?.removals.filter((removal) => removal.uninstallPlan.sharedInstallation ||
        !removal.uninstallPlan.allowed) ?? [];
    const canForceAgentOverwrite = agentOverwriteDialog?.removals.every((removal) => removal.uninstallPlan.allowed) ?? false;
    const canSafelyAgentOverwrite = agentOverwriteDialog?.removals.every((removal) => {
        const useDisable = removal.uninstallPlan.sharedInstallation ||
            !removal.uninstallPlan.allowed;
        return useDisable
            ? removal.disablePlan.allowed
            : removal.uninstallPlan.allowed;
    }) ?? false;
    const createCustomCategory = async (request: CreateCustomSkillCategoryRequest) => {
        const category = await api.createCustomSkillCategory(request);
        setCustomCategories((current) => [...current, category].sort((left, right) => left.name.localeCompare(right.name)));
        setExpandedNavigationGroups((current) => ({ ...current, custom: true }));
        setIsCreateCategoryOpen(false);
        setIsCreateProjectOpen(false);
        if (category.projectRoot) {
            setExpandedNavigationGroups((current) => ({ ...current, projects: true }));
            setLibraryVersion((version) => version + 1);
        }
        toggleView(`custom:${category.categoryId}`);
    };
    const requestDeleteCustomCategory = (category: CustomSkillCategory) => {
        setCategoryContextMenu(undefined);
        setCategoryDeleteDialog({ category, busy: false });
    };
    const deleteCustomCategory = async () => {
        if (!categoryDeleteDialog || categoryDeleteDialog.busy)
            return;
        const category = categoryDeleteDialog.category;
        setCategoryDeleteDialog({ category, busy: true });
        try {
            const deleted = await api.deleteCustomSkillCategory(category.categoryId);
            if (!deleted)
                throw new Error(tr("该自定义类别已不存在，请刷新后重试。"));
            setCustomCategories((current) => current.filter((item) => item.categoryId !== category.categoryId));
            const deletedView: ViewId = `custom:${category.categoryId}`;
            const removedAgentIds = agentWindows.ids.filter((id) => agentWindows.bindings[String(id)] === deletedView);
            if (removedAgentIds.length) {
                setAgentWindows((current) => {
                    const bindings = { ...current.bindings };
                    for (const id of removedAgentIds)
                        delete bindings[String(id)];
                    return {
                        ...current,
                        ids: current.ids.filter((id) => !removedAgentIds.includes(id)),
                        bindings,
                    };
                });
                setAgentHighlights((current) => {
                    const next = { ...current };
                    for (const id of removedAgentIds)
                        delete next[id];
                    return next;
                });
                if (activeAgentWindow !== null &&
                    removedAgentIds.includes(activeAgentWindow)) {
                    setActiveAgentWindow(null);
                }
            }
            if (activeView === `custom:${category.categoryId}`) {
                setActiveView(null);
                setIsSkillsListCollapsed(false);
                setSearchQuery("");
            }
            setCategoryDeleteDialog(undefined);
        }
        catch (error) {
            setCategoryDeleteDialog({
                category,
                busy: false,
                error: String(error),
            });
        }
    };
    const updateCustomCategoryState = (category: CustomSkillCategory) => {
        if (category.projectRoot)
            setLibraryVersion((version) => version + 1);
        setCustomCategories((current) => current.map((item) => item.categoryId === category.categoryId ? category : item));
    };
    const copySkillToCustomCategory = async (skill: InstalledSkill, categoryId: string) => {
        if (!categoryId || skill.isBuiltIn)
            return;
        const target = customCategories.find((category) => category.categoryId === categoryId);
        if (!target || target.skillIds.includes(skill.skillId))
            return;
        setSkillAction({ path: skill.path, message: "", type: "idle" });
        try {
            const result = await api.addSkillsToCustomCategory(categoryId, [skill.skillId]);
            updateCustomCategoryState(result.category);
            setSkillAction({
                path: null,
                message: result.addedCount > 0
                    ? tr("{0} 已加入 {1}。", skill.name, target.name) : tr("{0} 已包含该 Skill。", target.name),
                type: "success",
            });
        }
        catch (error) {
            setSkillAction({ path: null, message: String(error), type: "error" });
        }
    };
    const copyActiveCategory = async () => {
        if (!activeCustomCategory ||
            !effectiveCategoryCopyTarget ||
            categoryCopyBusy) {
            return;
        }
        const [targetType, targetId] = effectiveCategoryCopyTarget.split(":", 2);
        const targetLabel = categoryCopyOptions.find((option) => option.value === effectiveCategoryCopyTarget)?.label ?? tr("目标");
        if (targetType === "agent" && effectiveCategoryCopyMode === "overwrite") {
            const targetAgent = targetId as AgentId;
            const sourceIds = new Set(activeCategorySkills.map((skill) => skill.skillId));
            const targetSkills = (skillResult?.skills ?? []).filter((skill) => !skill.isBuiltIn && skill.enabledAgents.includes(targetAgent));
            const additions = activeCategorySkills.filter((skill) => !skill.enabledAgents.includes(targetAgent));
            const reenable = activeCategorySkills.filter((skill) => skill.enabledAgents.includes(targetAgent) &&
                skill.disabledAgents.includes(targetAgent));
            const removalSkills = targetSkills.filter((skill) => !sourceIds.has(skill.skillId));
            setCategoryCopyBusy(true);
            setSkillAction({ path: null, message: "", type: "idle" });
            try {
                const removals = await Promise.all(removalSkills.map(async (skill) => {
                    const [uninstallPlan, disablePlan] = await Promise.all([
                        api.prepareSkillAction(skill.skillId, targetAgent, "uninstall"),
                        api.prepareSkillAction(skill.skillId, targetAgent, "disable"),
                    ]);
                    return { skill, uninstallPlan, disablePlan };
                }));
                setAgentOverwriteDialog({
                    sourceName: activeCustomCategory.name,
                    targetAgent,
                    targetLabel,
                    additions,
                    reenable,
                    removals,
                    retainedCount: activeCategorySkills.length - additions.length - reenable.length,
                    busy: false,
                });
            }
            catch (error) {
                setSkillAction({
                    path: null,
                    message: tr("无法核对 Agent 覆盖范围：{0}", String(error)),
                    type: "error",
                });
            }
            finally {
                setCategoryCopyBusy(false);
            }
            return;
        }
        const targetCategory = targetType === "custom"
            ? customCategories.find((category) => category.categoryId === targetId)
            : undefined;
        if (targetType === "custom" && !targetCategory) {
            setSkillAction({
                path: null,
                message: tr("目标虚拟类别已不存在。"),
                type: "error",
            });
            return;
        }
        const sourceIds = new Set(activeCustomCategory.skillIds);
        const targetIds = new Set(targetCategory?.skillIds ?? []);
        const overwriteAdded = [...sourceIds].filter((id) => !targetIds.has(id)).length;
        const overwriteRemoved = [...targetIds].filter((id) => !sourceIds.has(id)).length;
        const confirmation = effectiveCategoryCopyMode === "overwrite"
            ? tr("用 {0} 覆盖 {1} 吗？将新增 {2} 个、移除 {3} 个成员。{4}", activeCustomCategory.name, targetLabel, overwriteAdded, overwriteRemoved, targetCategory?.projectRoot ? tr("将修改项目专用 Skills 文件，影响读取这些目录的 Agent；移除项保留在项目 .deadalus/project-copy 恢复目录。不会修改全局 Skills。") : tr("此操作只修改类别归属，不会删除 Skill 文件。")) : tr("将 {0} 中目标尚未包含的 Skills 增量复制到 {1} 吗？", activeCustomCategory.name, targetLabel);
        if (!window.confirm(confirmation)) {
            return;
        }
        setCategoryCopyBusy(true);
        setSkillAction({ path: null, message: "", type: "idle" });
        try {
            if (targetType === "custom") {
                if (!targetCategory)
                    throw new Error(tr("目标虚拟类别已不存在。"));
                if (effectiveCategoryCopyMode === "overwrite") {
                    const result = await api.replaceCustomCategoryMembers(activeCustomCategory.categoryId, targetId);
                    updateCustomCategoryState(result.category);
                    setSkillAction({
                        path: null,
                        message: tr("已覆盖 {0}：新增 {1} 个，移除 {2} 个，保留 {3} 个。", targetLabel, result.addedCount, result.removedCount, result.unchangedCount),
                        type: "success",
                    });
                    return;
                }
                const missing = activeCategorySkills.filter((skill) => !targetCategory.skillIds.includes(skill.skillId));
                if (missing.length === 0) {
                    setSkillAction({
                        path: null,
                        message: tr("{0} 已包含该类别的全部 Skills。", targetLabel),
                        type: "success",
                    });
                    return;
                }
                const result = await api.addSkillsToCustomCategory(targetId, missing.map((skill) => skill.skillId));
                updateCustomCategoryState(result.category);
                const alreadyPresent = activeCategorySkills.length - missing.length;
                setSkillAction({
                    path: null,
                    message: tr("已向 {0} 加入 {1} 个，跳过 {2} 个已有或不可加入的 Skills。", targetLabel, result.addedCount, alreadyPresent + result.skippedCount),
                    type: "success",
                });
            }
            else if (targetType === "agent") {
                const targetAgent = targetId as AgentId;
                const missing = activeCategorySkills.filter((skill) => !skill.enabledAgents.includes(targetAgent));
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
                    }
                    catch {
                        failed += 1;
                    }
                }
                const skipped = activeCategorySkills.length - missing.length;
                setSkillAction({
                    path: null,
                    message: tr("已复制 {0} 个到 {1}，跳过 {2} 个已有 Skills{3}。", copied, targetLabel, skipped, failed ? tr("，{0} 个复制失败", failed) : ""),
                    type: failed ? "error" : "success",
                });
                if (copied > 0)
                    setLibraryVersion((version) => version + 1);
            }
        }
        catch (error) {
            setSkillAction({ path: null, message: String(error), type: "error" });
        }
        finally {
            setCategoryCopyBusy(false);
        }
    };
    const executeAgentOverwrite = async (sharedStrategy: "uninstall" | "disable") => {
        const dialog = agentOverwriteDialog;
        if (!dialog || dialog.busy)
            return;
        setAgentOverwriteDialog({ ...dialog, busy: true, error: undefined });
        let copied = 0;
        let enabled = 0;
        let removed = 0;
        let disabled = 0;
        try {
            for (const skill of dialog.additions) {
                if (!skill.libraryPath || !skill.inLibrary) {
                    throw new Error(tr("{0} 没有可复制的 All Skills 后备路径。", skill.name));
                }
                await api.copyLibrarySkillToAgent(skill.libraryPath, dialog.targetAgent);
                copied += 1;
            }
            for (const skill of dialog.reenable) {
                await api.enableSkillForAgent(skill.skillId, dialog.targetAgent);
                enabled += 1;
            }
            for (const removal of dialog.removals) {
                const useDisable = sharedStrategy === "disable" &&
                    (removal.uninstallPlan.sharedInstallation ||
                        !removal.uninstallPlan.allowed);
                const plan = useDisable ? removal.disablePlan : removal.uninstallPlan;
                if (!plan.allowed) {
                    throw new Error(tr("{0} 无法{1}：{2}", removal.skill.name, useDisable ? tr("禁用") : tr("卸载"), plan.reason ?? tr("未通过操作预检")));
                }
                if (useDisable) {
                    await api.disableSkillForAgent(removal.skill.skillId, dialog.targetAgent);
                    disabled += 1;
                }
                else {
                    await api.uninstallSkill(removal.skill.skillId, dialog.targetAgent);
                    removed += 1;
                }
            }
            setSkillAction({
                path: null,
                message: tr("已覆盖 {0}：新增 {1} 个，解禁 {2} 个，卸载 {3} 个{4}；内置 Skills 保持不变。", dialog.targetLabel, copied, enabled, removed, disabled ? tr("，禁用共享项 {0} 个", disabled) : ""),
                type: "success",
            });
            setAgentOverwriteDialog(undefined);
            setLibraryVersion((version) => version + 1);
        }
        catch (error) {
            setAgentOverwriteDialog({
                ...dialog,
                busy: false,
                error: tr("覆盖中断：{0} 已完成新增 {1}、解禁 {2}、卸载 {3}、禁用 {4}。", String(error), copied, enabled, removed, disabled),
            });
        }
    };
    const copyRecommendedSkills = async (skillIds: string[], target: string, mode: CategoryCopyMode) => {
        const [kind, targetId] = target.split(":", 2);
        const source = (skillResult?.skills ?? []).filter((skill) => skillIds.includes(skill.skillId) && !skill.isBuiltIn);
        if (source.length !== skillIds.length)
            throw new Error(tr("推荐结果已变化或包含内置 Skill，请重新检索。"));
        if (kind === "custom") {
            const category = customCategories.find((item) => item.categoryId === targetId);
            if (!category)
                throw new Error(tr("目标类别不存在。"));
            if (!window.confirm(mode === "overwrite" ? tr("用 {0} 个推荐 Skills 覆盖 {1}？目标中其他成员将被移除。{2}", source.length, category.name, category.projectRoot ? tr("将修改项目专用文件，影响共享目录的 Agent；移除项保留在 .deadalus/project-copy，不修改全局 Skills。") : "") : tr("将 {0} 个推荐 Skills 增量复制到 {1}？", source.length, category.name)))
                return false;
            const result = mode === "overwrite"
                ? await api.replaceCustomCategoryWithSkills(targetId, skillIds)
                : await api.addSkillsToCustomCategory(targetId, skillIds);
            updateCustomCategoryState(result.category);
            setGraphRevision((value) => value + 1);
            return true;
        }
        if (kind !== "agent" || !agents.some((item) => item.id === targetId))
            throw new Error(tr("未知复制目标。"));
        const agent = targetId as AgentId;
        const additions = source.filter((skill) => !skill.enabledAgents.includes(agent));
        const reenable = source.filter((skill) => skill.enabledAgents.includes(agent) && skill.disabledAgents.includes(agent));
        if (mode === "overwrite") {
            const selected = new Set(skillIds);
            const existing = (skillResult?.skills ?? []).filter((skill) => !skill.isBuiltIn && skill.enabledAgents.includes(agent) && !selected.has(skill.skillId));
            const removals = await Promise.all(existing.map(async (skill) => ({
                skill,
                uninstallPlan: await api.prepareSkillAction(skill.skillId, agent, "uninstall"),
                disablePlan: await api.prepareSkillAction(skill.skillId, agent, "disable"),
            })));
            setAgentOverwriteDialog({
                sourceName: tr("Agent 推荐结果"), targetAgent: agent, targetLabel: agent,
                additions, reenable, removals,
                retainedCount: source.length - additions.length - reenable.length,
                busy: false,
            });
            return true;
        }
        if (!window.confirm(tr("将 {0} 个推荐 Skills 中缺少的成员增量复制到 {1}？", source.length, agent)))
            return false;
        for (const skill of additions) {
            if (!skill.libraryPath || !skill.inLibrary)
                throw new Error(tr("{0} 没有可复制的后备文件。", skill.name));
            await api.copyLibrarySkillToAgent(skill.libraryPath, agent);
        }
        for (const skill of reenable)
            await api.enableSkillForAgent(skill.skillId, agent);
        setLibraryVersion((value) => value + 1);
        return true;
    };
    const toggleSkillExpansion = (path: string) => {
        setExpandedSkills((current) => {
            const next = new Set(current);
            if (next.has(path)) {
                next.delete(path);
            }
            else {
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
    const runSkillAction = async (path: string, action: () => Promise<unknown>, successMessage: string, onError?: (message: string) => void) => {
        setSkillAction({ path, message: "", type: "idle" });
        try {
            await action();
            setSkillAction({ path: null, message: successMessage, type: "success" });
            setLibraryVersion((version) => version + 1);
        }
        catch (error) {
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
        if (!skill.libraryPath)
            return;
        const firstAvailable = agents.find((agent) => !skill.enabledAgents.includes(agent.id))?.id;
        const target = installTargets[skill.path] ?? firstAvailable;
        if (!target) {
            setCopyFailureDialog({
                skillName: skill.name,
                targetName: tr("目标 Agent"),
                message: tr("没有可用的目标 Agent，或该 Skill 已存在于所有支持的 Agent 中。"),
            });
            return;
        }
        const targetName = agents.find((agent) => agent.id === target)?.label ?? target;
        if (!window.confirm(tr("将 {0} 复制到 {1} 吗？", skill.name, targetName))) {
            return;
        }
        runSkillAction(skill.path, () => api.copyLibrarySkillToAgent(skill.libraryPath!, target), tr("{0} 已复制到 {1}。", skill.name, targetName), (message) => setCopyFailureDialog({
            skillName: skill.name,
            targetName,
            message,
        }));
    };
    const prepareSkillAction = async (skill: InstalledSkill, action: "uninstall" | "disable") => {
        if (!activeView)
            return;
        setSkillActionDialog({ skill, loading: true });
        try {
            const plan = await api.prepareSkillAction(skill.skillId, activeView, action);
            setSkillActionDialog({ skill, plan, loading: false });
        }
        catch (error) {
            setSkillActionDialog({ skill, loading: false, error: String(error) });
        }
    };
    const executePreparedAction = async () => {
        const dialog = skillActionDialog;
        if (!dialog?.plan?.allowed || !dialog.skill)
            return;
        const { plan, skill } = dialog;
        setSkillActionDialog({ ...dialog, loading: true });
        await runSkillAction(skill.path, () => plan.action === "uninstall"
            ? api.uninstallSkill(skill.skillId, plan.viewId)
            : api.disableSkillForAgent(skill.skillId, plan.viewId), plan.action === "uninstall"
            ? tr("{0} 已从 {1} 卸载。", skill.name, plan.viewId === "all" ? tr("所有 Skills") : plan.viewId) : tr("{0} 已在当前 Agent 中禁用。", skill.name));
        setSkillActionDialog(null);
    };
    const disableInstead = async () => {
        const skill = skillActionDialog?.skill;
        const viewId = skillActionDialog?.plan?.viewId;
        if (!skill || !viewId || viewId === "all")
            return;
        setSkillActionDialog({ skill, loading: true });
        try {
            const plan = await api.prepareSkillAction(skill.skillId, viewId, "disable");
            setSkillActionDialog({ skill, plan, loading: false });
        }
        catch (error) {
            setSkillActionDialog({ skill, loading: false, error: String(error) });
        }
    };
    const enableSkill = (skill: InstalledSkill, agent: AgentId) => {
        runSkillAction(skill.path, () => api.enableSkillForAgent(skill.skillId, agent), tr("{0} 已在 {1} 中解禁。", skill.name, agents.find((item) => item.id === agent)?.label));
    };
    const restoreBackup = (skill: InstalledSkill) => {
        runSkillAction(skill.path, () => api.restoreSkillBackup(skill.skillId), tr("{0} 已重新加入所有 Skills 后备库。", skill.name));
    };
    return (<div className={`app-shell ${activeView ? "app-shell--skills-open" : ""} ${activeView && isSkillsListCollapsed ? "app-shell--skills-collapsed" : ""} ${isAssistantOpen ? "app-shell--assistant-open" : ""}`} data-page="a1" style={shellStyle}>
      <aside className="side-platform" aria-label={tr("左侧平台")}>
        <button ref={dButtonRef} className="brand-mark" type="button" aria-label={tr("打开 API Key 管理")} title={tr("API Key 管理")} onClick={() => setIsKeyDialogOpen(true)}>
          <img src="/deadalus-icon.svg" alt="" aria-hidden="true"/>
        </button>

        <nav className="agent-actions" aria-label={tr("Skill 类别")}>
          <button className={`agent-button agent-button--all ${activeView === "all" ? "agent-button--active" : ""}`} type="button" aria-label={tr("所有 Skills")} aria-pressed={activeView === "all"} title={tr("所有 Skills")} onClick={() => toggleView("all")}>
            <svg className="folder-icon" viewBox="0 0 24 24" aria-hidden="true">
              <path d="M3.75 6.75A1.75 1.75 0 0 1 5.5 5h4.1c.5 0 .97.21 1.3.58l1.05 1.17h6.55a1.75 1.75 0 0 1 1.75 1.75v8A1.75 1.75 0 0 1 18.5 18h-13a1.75 1.75 0 0 1-1.75-1.75v-9.5Z" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round"/>
            </svg>
            <span className="agent-button__full">{tr("所有 Skills")}</span>
            <span className="agent-button__short">{tr("全部")}</span>
          </button>

          <NavigationGroupToggle id="agents" label={tr("Agent 全局")} shortLabel="Agents" expanded={expandedNavigationGroups.agents} onToggle={() => toggleNavigationGroup("agents")}>
            {agents.map((agent) => (<button key={agent.id} className={`agent-button ${activeView === agent.id ? "agent-button--active" : ""}`} type="button" aria-label={`${agent.label} Skills`} aria-pressed={activeView === agent.id} title={`${agent.label} Skills`} onClick={() => toggleView(agent.id)}>
                <span className="agent-button__full">{agent.label}</span>
                <span className="agent-button__short">{agent.shortLabel}</span>
              </button>))}
          </NavigationGroupToggle>

          <NavigationGroupToggle id="custom" label={tr("自定义类别")} shortLabel={tr("自定义")} expanded={expandedNavigationGroups.custom} onToggle={() => toggleNavigationGroup("custom")}>
            {customCategories.filter((category) => !category.projectRoot).length === 0 ? (<div className="navigation-group__empty">{tr("暂无自定义类别")}</div>) : (customCategories.filter((category) => !category.projectRoot).map((category) => {
            const viewId = `custom:${category.categoryId}` as const;
            return (<button key={category.categoryId} className={`agent-button category-button ${activeView === viewId ? "agent-button--active" : ""}`} type="button" aria-label={`${category.name} Skills`} aria-pressed={activeView === viewId} title={category.description} onClick={() => toggleView(viewId)} onContextMenu={(event) => {
                    event.preventDefault();
                    setCategoryContextMenu({
                        category,
                        x: Math.min(event.clientX, window.innerWidth - 176),
                        y: Math.min(event.clientY, window.innerHeight - 64),
                    });
                }}>
                    <span className="category-button__dot" style={{ backgroundColor: category.color }} aria-hidden="true"/>
                    <span className="agent-button__full">{category.name}</span>
                  </button>);
        }))}
            <button className="agent-button agent-button--add" type="button" aria-label={tr("新建自定义类别")} title={tr("新建自定义类别")} onClick={() => setIsCreateCategoryOpen(true)}>
              <span aria-hidden="true">＋</span>
              <span className="agent-button__full">{tr("新建类别")}</span>
            </button>
          </NavigationGroupToggle>

          <NavigationGroupToggle id="projects" label={tr("项目")} shortLabel={tr("项目")} expanded={expandedNavigationGroups.projects} onToggle={() => toggleNavigationGroup("projects")}>
            {!customCategories.some((category) => category.projectRoot) && <div className="navigation-group__empty">{tr("暂无项目")}</div>}
            {customCategories.filter((category) => category.projectRoot).map((category) => (<button key={category.categoryId} type="button" className={`agent-button category-button ${activeView === `custom:${category.categoryId}` ? "agent-button--active" : ""}`} aria-label={`${category.name} Skills`} aria-pressed={activeView === `custom:${category.categoryId}`} title={`${category.description}\n${category.projectRoot}`} onClick={() => { toggleView(`custom:${category.categoryId}`); setLibraryVersion((version) => version + 1); }} onContextMenu={(event) => { event.preventDefault(); setCategoryContextMenu({ category, x: Math.min(event.clientX, window.innerWidth - 176), y: Math.min(event.clientY, window.innerHeight - 64) }); }}>
                <span className="category-button__dot" style={{ backgroundColor: category.color }} aria-hidden="true"/>
                <span className="agent-button__full">{category.name}</span>
              </button>))}
            <button type="button" className="agent-button agent-button--add" aria-label={tr("新建项目")} title={tr("新建项目")} onClick={() => setIsCreateProjectOpen(true)}>
              <span aria-hidden="true">＋</span><span className="agent-button__full">{tr("新建项目")}</span>
            </button>
          </NavigationGroupToggle>
        </nav>

      </aside>

      {categoryContextMenu && (<div className="category-context-layer" role="presentation" onMouseDown={() => setCategoryContextMenu(undefined)} onContextMenu={(event) => {
                event.preventDefault();
                setCategoryContextMenu(undefined);
            }}>
          <div className="category-context-menu" role="menu" aria-label={tr("{0} 类别操作", categoryContextMenu.category.name)} style={{ left: categoryContextMenu.x, top: categoryContextMenu.y }} onMouseDown={(event) => event.stopPropagation()}>
            <button ref={categoryMenuButtonRef} type="button" role="menuitem" onClick={() => requestDeleteCustomCategory(categoryContextMenu.category)}>
              <span aria-hidden="true">×</span>{tr("删除该类别")}</button>
          </div>
        </div>)}

      {agentWindowContextMenu && (<div className="category-context-layer" role="presentation" onMouseDown={() => setAgentWindowContextMenu(undefined)} onContextMenu={(event) => {
                event.preventDefault();
                setAgentWindowContextMenu(undefined);
            }}>
          <div className="category-context-menu" role="menu" aria-label={tr("Agent {0} 窗口操作", agentWindowContextMenu.windowId)} style={{ left: agentWindowContextMenu.x, top: agentWindowContextMenu.y }} onMouseDown={(event) => event.stopPropagation()}>
            <button ref={agentWindowMenuButtonRef} type="button" role="menuitem" onClick={() => {
                setAgentWindowDeleteDialog(agentWindowContextMenu.windowId);
                setAgentWindowContextMenu(undefined);
            }}>
              <span aria-hidden="true">×</span>{tr("删除该 Agent 窗口")}</button>
          </div>
        </div>)}

      <aside className={`skills-platform${isSkillsListCollapsed ? " skills-platform--collapsed" : ""}`} aria-label="Agent Skills" aria-hidden={!activeView}>
        {activeView && (<>
            <header className="skills-header">
              {!isSkillsListCollapsed && (<div>
                  <p className="eyebrow">
                    {activeView === "all"
                    ? "SKILL LIBRARY"
                    : activeCustomCategory
                        ? "CUSTOM CATEGORY"
                        : "AGENT SKILLS"}
                  </p>
                  <h2>
                    {activeView === "all"
                    ? tr("所有 Skills") : activeCustomCategory?.name ??
                    agents.find((agent) => agent.id === activeView)?.label}
                  </h2>
                  {activeCustomCategory && (<p className="custom-category-description">
                      {activeCustomCategory.description}
                    </p>)}
                </div>)}
              <div className="skills-header__actions">
                {!isSkillsListCollapsed && (<span className="skill-count">
                    {skillState === "ready" ? visibleSkills.length : "—"}
                  </span>)}
                <button className="skills-collapse-toggle" type="button" aria-label={isSkillsListCollapsed
                ? tr("展开 Skills 列表") : tr("收起 Skills 列表")} aria-expanded={!isSkillsListCollapsed} title={isSkillsListCollapsed
                ? tr("展开 Skills 列表") : tr("收起 Skills 列表")} onClick={() => setIsSkillsListCollapsed((current) => !current)}>
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path d={isSkillsListCollapsed
                ? "M9 6l6 6-6 6"
                : "M15 6l-6 6 6 6"} fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/>
                  </svg>
                </button>
              </div>
            </header>

            {!isSkillsListCollapsed && activeCustomCategory && (<div className="custom-category-copybar">
                <label>
                  <span>{tr("复制类别到")}</span>
                  <select aria-label={tr("选择 {0} 的复制目标", activeCustomCategory.name)} value={effectiveCategoryCopyTarget} disabled={categoryCopyBusy || categoryCopyOptions.length === 0} onChange={(event) => setCategoryCopyTarget(event.currentTarget.value)}>
                    {categoryCopyOptions.map((option) => (<option key={option.value} value={option.value}>
                        {option.label}
                      </option>))}
                  </select>
                </label>
                <label>
                  <span>{tr("复制方式")}</span>
                  <select aria-label={tr("选择 {0} 的复制方式", activeCustomCategory.name)} value={effectiveCategoryCopyMode} disabled={categoryCopyBusy} title={tr("增量复制只补齐缺少项；覆盖复制会同步新增和移除项")} onChange={(event) => setCategoryCopyMode(event.currentTarget.value as CategoryCopyMode)}>
                    <option value="incremental">{tr("增量复制")}</option>
                    <option value="overwrite">{tr("覆盖复制")}</option>
                  </select>
                </label>
                <button type="button" disabled={categoryCopyBusy ||
                    (effectiveCategoryCopyMode === "incremental" &&
                        activeCategorySkills.length === 0) ||
                    !effectiveCategoryCopyTarget} onClick={copyActiveCategory}>
                  {categoryCopyBusy
                    ? effectiveCategoryCopyMode === "overwrite" &&
                        effectiveCategoryCopyTarget.startsWith("agent:")
                        ? tr("正在核对…") : tr("正在复制…")
                    : effectiveCategoryCopyMode === "overwrite"
                        ? effectiveCategoryCopyTarget.startsWith("agent:")
                            ? tr("覆盖目标 Agent") : tr("覆盖目标类别")
                        : tr("增量复制")}
                </button>
              </div>)}

            {!isSkillsListCollapsed && (<>
            <div className="skills-content">
              {(activeView === "all" || activeCustomCategory) && (<div className="all-skills-tools">
                  <label className="skill-search">
                    <span className="sr-only">{tr("搜索 Skills")}</span>
                    <input type="search" value={searchQuery} placeholder={tr("搜索名称或描述")} onChange={(event) => setSearchQuery(event.currentTarget.value)}/>
                  </label>
                  {activeView === "all" && <div className="agent-filters" aria-label={tr("按 Agent 筛选")}>
                    <button type="button" className={agentFilter === "all" ? "is-active" : ""} onClick={() => setAgentFilter("all")}>{tr("全部")}</button>
                    {agents.map((agent) => (<button key={agent.id} type="button" className={agentFilter === agent.id ? "is-active" : ""} onClick={() => setAgentFilter(agent.id)}>
                        {agent.shortLabel}
                      </button>))}
                  </div>}
                </div>)}
              {skillAction.message && (<div className={`skill-action-feedback skill-action-feedback--${skillAction.type}`} role="status">
                  {skillAction.message}
                </div>)}
              {skillResult && skillResult.warnings.length > 0 && (<div className="skill-warning" role="status">
                  {activeView === "all"
                        ? tr("{0} 个来源或条目读取失败，其余结果已保留。", skillResult.warnings.length) : skillResult.warnings.join("；")}
                </div>)}
              {skillState === "loading" && (<div className="skills-state">
                  <span className="spinner"/>
                  <p>{tr("正在读取官方目录…")}</p>
                </div>)}
              {skillState === "error" && (<div className="skills-state skills-state--error">
                  <p>{tr("读取失败")}</p>
                  <span>{skillError}</span>
                </div>)}
              {skillState === "ready" &&
                    skillResult &&
                    visibleSkills.length === 0 && (<div className="skills-state">
                    <p>
                      {skillResult.skills.length === 0
                        ? tr("尚未发现 Skills") : tr("没有匹配的 Skills")}
                    </p>
                    <span>
                      {activeView === "all"
                        ? tr("当前统一快照中没有匹配项") : activeCustomCategory
                        ? tr("该自定义类别中没有匹配项") : tr("当前统一快照中该 Agent 没有匹配项")}
                    </span>
                  </div>)}
              {skillState === "ready" &&
                    skillResult &&
                    visibleSkills.length > 0 && (<ul className="skill-list">
                    {visibleSkills.map((skill) => {
                        const isExpanded = expandedSkills.has(skill.path);
                        const projectShared = activeCustomCategory?.sharedSkillIds?.includes(skill.skillId);
                        const isDisabled = Boolean(activeAgent && skill.disabledAgents.includes(activeAgent));
                        const availableAgents = agents.filter((agent) => !skill.enabledAgents.includes(agent.id));
                        const availableCustomCategories = skill.isBuiltIn
                            ? []
                            : customCategories.filter((category) => !category.skillIds.includes(skill.skillId));
                        const selectedCustomCategoryId = availableCustomCategories.some((category) => category.categoryId === customCategoryTargets[skill.path])
                            ? customCategoryTargets[skill.path]
                            : availableCustomCategories[0]?.categoryId ?? "";
                        return (<li key={skill.skillId} className={`skill-item ${isExpanded ? "skill-item--expanded" : ""}${isDisabled ? " skill-item--disabled" : ""}${projectShared ? " skill-item--project-shared" : ""}`}>
                        <span className="skill-item__mark"/>
                        <div className="skill-item__body">
                          <div className="skill-item__title">
                            <div className="skill-item__identity">
                              <p>{skill.name}</p>
                              {projectShared && <span className="skill-item__badge project-shared-warning" title={tr("此目录会被多个 Agent 读取，项目内修改会同时影响它们。")}>{tr("目录交叉 · 多 Agent 共享")}</span>}
                              {skill.isBuiltIn && (<span className="skill-item__badge">{tr("内置")}</span>)}
                              {isDisabled && (<span className="skill-item__badge skill-item__badge--disabled">{tr("已禁用")}</span>)}
                            </div>
                            {!activeCustomCategory && <div className="skill-item__quick-actions">
                              <button className="skill-quick-action skill-quick-action--uninstall" type="button" aria-label={tr("{0} {1}", activeView === "all" ? tr("删除后备副本") : tr("卸载"), skill.name)} title={activeView === "all" ? tr("删除后备副本") : tr("从当前 Agent 卸载")} disabled={skillAction.path === skill.path || (activeView === "all" && !skill.inLibrary)} onClick={() => prepareSkillAction(skill, "uninstall")}>
                                ×
                              </button>
                              {activeView !== "all" && activeAgent && (<button className={`skill-quick-action ${isDisabled ? "skill-quick-action--enable" : "skill-quick-action--disable"}`} type="button" aria-label={tr("{0} {1}", isDisabled ? tr("解禁") : tr("禁用当前 Agent 中的"), skill.name)} title={isDisabled ? tr("解禁") : tr("禁用当前")} disabled={skillAction.path === skill.path} onClick={() => isDisabled
                                        ? enableSkill(skill, activeAgent)
                                        : prepareSkillAction(skill, "disable")}>
                                  {isDisabled ? "✓" : "−"}
                                </button>)}
                              {activeView === "all" && skill.backupSuppressed && (<button className="skill-quick-action skill-quick-action--restore" type="button" aria-label={tr("重新备份 {0}", skill.name)} title={tr("从 Agent 重新添加到所有 Skills")} disabled={skillAction.path === skill.path} onClick={() => restoreBackup(skill)}>
                                  ↥
                                </button>)}
                            </div>}
                          </div>
                          {skill.description && <DisplaySummary text={skill.description}/>}
                          {activeAgent && (<code title={skill.path}>{skill.path}</code>)}
                          <button className="skill-expand-button" type="button" aria-expanded={isExpanded} aria-label={tr("{0} {1} 的完整信息", isExpanded ? tr("收起") : tr("展开"), skill.name)} onClick={() => toggleSkillExpansion(skill.path)}>
                            {isExpanded ? tr("收起") : tr("展开")}
                          </button>
                          {activeView === "all" && isExpanded && (<div className="skill-library-actions">
                              {skill.inLibrary &&
                                    skill.libraryPath &&
                                    availableAgents.length > 0 && (<div className="skill-install-control">
                                    <select aria-label={tr("选择 {0} 的安装目标", skill.name)} value={installTargets[skill.path] ??
                                        availableAgents[0].id} onChange={(event) => {
                                        const target = event.currentTarget.value as AgentId;
                                        setInstallTargets((current) => ({
                                            ...current,
                                            [skill.path]: target,
                                        }));
                                    }}>
                                      {availableAgents.map((agent) => (<option key={agent.id} value={agent.id}>
                                          {agent.shortLabel}
                                        </option>))}
                                    </select>
                                    <button type="button" disabled={skillAction.path === skill.path} onClick={() => installToAgent(skill)}>{tr("复制到 Agent")}</button>
                                  </div>)}
                              <div className="skill-install-control skill-install-control--category">
                                <select aria-label={tr("选择 {0} 的虚拟类别目标", skill.name)} value={selectedCustomCategoryId} disabled={availableCustomCategories.length === 0} onChange={(event) => {
                                    const categoryId = event.currentTarget.value;
                                    setCustomCategoryTargets((current) => ({
                                        ...current,
                                        [skill.path]: categoryId,
                                    }));
                                }}>
                                  {availableCustomCategories.length === 0 ? (<option value="">
                                      {skill.isBuiltIn
                                        ? tr("内置 Skills 不加入虚拟类别") : tr("所有类别均已包含")}
                                    </option>) : (availableCustomCategories.map((category) => (<option key={category.categoryId} value={category.categoryId}>
                                        {category.name}
                                      </option>)))}
                                </select>
                                <button type="button" disabled={skillAction.path === skill.path ||
                                    !selectedCustomCategoryId} onClick={() => copySkillToCustomCategory(skill, selectedCustomCategoryId)}>{tr("复制到类别")}</button>
                              </div>
                            </div>)}
                        </div>
                      </li>);
                    })}
                  </ul>)}
            </div>

            {skillResult && (<footer className="skills-footer">
                <span>{tr("统一 All Skills 快照")}</span>
                <span title={skillResult.searchedPaths.join("\n")}>
                  {skillResult.searchedPaths.length}{tr("个扫描目录")}</span>
              </footer>)}
              </>)}
          </>)}
      </aside>

      {skillActionDialog && (<div className="skill-action-backdrop" role="presentation" onMouseDown={() => !skillActionDialog.loading && setSkillActionDialog(null)}>
          <section className="skill-action-dialog" role="alertdialog" aria-modal="true" aria-labelledby="skill-action-title" onMouseDown={(event) => event.stopPropagation()}>
            <header>
              <div>
                <p className="eyebrow">{tr("Skill 管理")}</p>
                <h3 id="skill-action-title">
                  {skillActionDialog.plan?.action === "disable" ? tr("禁用当前 Agent") : tr("确认卸载")}
                </h3>
              </div>
              <button type="button" className="modal-close" aria-label={tr("关闭")} disabled={skillActionDialog.loading} onClick={() => setSkillActionDialog(null)}>×</button>
            </header>
            {skillActionDialog.loading && <div className="skill-action-dialog__state"><span className="spinner"/>{tr("正在核对实际影响范围…")}</div>}
            {skillActionDialog.error && <p className="skill-action-dialog__error">{skillActionDialog.error}</p>}
            {skillActionDialog.plan && !skillActionDialog.loading && (<>
                <strong>{skillActionDialog.plan.skillName}</strong>
                <p>
                  {skillActionDialog.plan.action === "disable"
                    ? tr("只禁用 {0}，不会删除 Skill 文件或向量。", skillActionDialog.plan.viewId) : skillActionDialog.plan.viewId === "all"
                    ? tr("后备副本将被删除，并停止在后续扫描中自动重新备份。") : tr("将从 {0} 的实际安装位置卸载。", skillActionDialog.plan.viewId)}
                </p>
                {skillActionDialog.plan.sharedInstallation && (<div className="skill-action-dialog__impact">{tr("此路径同时被以下 Agent 使用：")}{skillActionDialog.plan.affectedAgents.join("、")}{tr("。继续卸载会同时影响它们。")}</div>)}
                {skillActionDialog.plan.pluginOperation && (<div className="skill-action-dialog__impact">{tr("该 Skill 由插件提供，卸载可能同时影响插件内其他 Skills。")}</div>)}
                {skillActionDialog.plan.symbolicLinkOnly && (<div className="skill-action-dialog__note">{tr("只会移除符号链接，不会删除链接目标内容。")}</div>)}
                {skillActionDialog.plan.targetPaths.length > 0 && (<details>
                    <summary>{tr("实际目标 ·")}{skillActionDialog.plan.targetPaths.length}</summary>
                    {skillActionDialog.plan.targetPaths.map((path) => <code key={path}>{path}</code>)}
                  </details>)}
                {!skillActionDialog.plan.allowed && (<p className="skill-action-dialog__error">{skillActionDialog.plan.reason}</p>)}
                <footer>
                  {skillActionDialog.plan.action === "uninstall" && skillActionDialog.plan.viewId !== "all" && (<button type="button" className="warning-button" onClick={disableInstead}>{tr("禁用当前")}</button>)}
                  <button type="button" onClick={() => setSkillActionDialog(null)}>{tr("取消")}</button>
                  <button type="button" className={skillActionDialog.plan.action === "uninstall" ? "danger-button" : "warning-button"} disabled={!skillActionDialog.plan.allowed} onClick={executePreparedAction}>
                    {skillActionDialog.plan.action === "uninstall" ? tr("确认卸载") : tr("确认禁用")}
                  </button>
                </footer>
              </>)}
          </section>
        </div>)}

      {agentOverwriteDialog && (<div className="skill-action-backdrop" role="presentation" onMouseDown={() => !agentOverwriteDialog.busy && setAgentOverwriteDialog(undefined)}>
          <section className="skill-action-dialog agent-overwrite-dialog" role="alertdialog" aria-modal="true" aria-labelledby="agent-overwrite-title" onMouseDown={(event) => event.stopPropagation()}>
            <header>
              <div>
                <p className="eyebrow">{tr("Agent 覆盖")}</p>
                <h3 id="agent-overwrite-title">{tr("覆盖 {0}", agentOverwriteDialog.targetLabel)}
                </h3>
              </div>
              <button type="button" className="modal-close" aria-label={tr("关闭 Agent 覆盖确认")} disabled={agentOverwriteDialog.busy} onClick={() => setAgentOverwriteDialog(undefined)}>
                ×
              </button>
            </header>
            <strong>{agentOverwriteDialog.sourceName}</strong>
            <p>{tr("将目标 Agent 的普通 Skills 同步为该虚拟类别。内置 Skills 不参与覆盖并保持原样。")}</p>
            <div className="agent-overwrite-dialog__counts" aria-label={tr("覆盖变更摘要")}>
              <span>{tr("新增")}{agentOverwriteDialog.additions.length}</span>
              <span>{tr("解禁")}{agentOverwriteDialog.reenable.length}</span>
              <span>{tr("移除")}{agentOverwriteDialog.removals.length}</span>
              <span>{tr("保留")}{agentOverwriteDialog.retainedCount}</span>
            </div>
            {agentOverwriteRiskRemovals.length > 0 && (<div className="skill-action-dialog__impact">
                <strong>
                  {agentOverwriteRiskRemovals.length}{tr("个待移除 Skill 会影响其他 Agent 或不能直接卸载。")}</strong>
                <p>{tr("选择“禁用风险项”只在当前 Agent 中禁用这些 Skill；选择“仍然删除”会按实际共享路径卸载，可能同时影响其他 Agent。")}</p>
                <details>
                  <summary>{tr("查看影响范围")}</summary>
                  {agentOverwriteRiskRemovals.map((removal) => (<div className="agent-overwrite-dialog__risk" key={removal.skill.skillId}>
                      <span>{removal.skill.name}</span>
                      <small>
                        {removal.uninstallPlan.sharedInstallation
                        ? tr("共享：{0}", removal.uninstallPlan.affectedAgents.join("、")) : removal.uninstallPlan.reason ?? tr("需要改为禁用")}
                      </small>
                    </div>))}
                </details>
              </div>)}
            {agentOverwriteDialog.error && (<p className="skill-action-dialog__error" role="alert">
                {agentOverwriteDialog.error}
              </p>)}
            {agentOverwriteDialog.busy && (<div className="skill-action-dialog__state">
                <span className="spinner"/>{tr("正在同步 Agent Skills…")}</div>)}
            <footer>
              <button type="button" disabled={agentOverwriteDialog.busy} onClick={() => setAgentOverwriteDialog(undefined)}>{tr("取消")}</button>
              {agentOverwriteRiskRemovals.length > 0 && (<button type="button" className="warning-button" disabled={agentOverwriteDialog.busy || !canSafelyAgentOverwrite} title={canSafelyAgentOverwrite
                    ? tr("独立项卸载，共享或不可卸载项仅禁用当前 Agent") : tr("至少一个风险项无法只在当前 Agent 中禁用")} onClick={() => executeAgentOverwrite("disable")}>{tr("禁用风险项")}</button>)}
              <button type="button" className="danger-button" disabled={agentOverwriteDialog.busy || !canForceAgentOverwrite} title={canForceAgentOverwrite
                ? tr("执行新增、解禁与物理卸载") : tr("至少一个 Skill 未通过卸载预检")} onClick={() => executeAgentOverwrite("uninstall")}>
                {agentOverwriteRiskRemovals.length > 0
                ? tr("仍然删除") : tr("确认覆盖")}
              </button>
            </footer>
          </section>
        </div>)}

      {copyFailureDialog && (<div className="skill-action-backdrop" role="presentation" onMouseDown={() => setCopyFailureDialog(null)}>
          <section className="skill-action-dialog copy-failure-dialog" role="alertdialog" aria-modal="true" aria-labelledby="copy-failure-title" onMouseDown={(event) => event.stopPropagation()}>
            <header>
              <div>
                <p className="eyebrow">{tr("Skill 复制")}</p>
                <h3 id="copy-failure-title">{tr("复制失败")}</h3>
              </div>
              <button type="button" className="modal-close" aria-label={tr("关闭复制失败提醒")} onClick={() => setCopyFailureDialog(null)}>
                ×
              </button>
            </header>
            <strong>{copyFailureDialog.skillName}</strong>
            <p>{tr("未能复制到")}{copyFailureDialog.targetName}。</p>
            <div className="skill-action-dialog__impact copy-failure-dialog__reason">
              {copyFailureDialog.message}
            </div>
            <p className="copy-failure-dialog__hint">{tr("可能原因包括目标已存在、Skill 包结构无效、包含不可复制的链接、目录权限不足，或目标 Agent 不兼容。")}</p>
            <footer>
              <button type="button" onClick={() => setCopyFailureDialog(null)}>{tr("知道了")}</button>
            </footer>
          </section>
        </div>)}

      {categoryDeleteDialog && (<div className="skill-action-backdrop" role="presentation">
          <section className="skill-action-dialog category-delete-dialog" role="alertdialog" aria-modal="true" aria-labelledby="category-delete-title">
            <header>
              <div>
                <p className="eyebrow">{tr("自定义分类")}</p>
                <h3 id="category-delete-title">{tr("删除自定义类别")}</h3>
              </div>
              <button type="button" className="modal-close" aria-label={tr("关闭删除类别确认")} disabled={categoryDeleteDialog.busy} onClick={() => setCategoryDeleteDialog(undefined)}>
                ×
              </button>
            </header>
            <strong>{categoryDeleteDialog.category.name}</strong>
            <p>{categoryDeleteDialog.category.projectRoot ? tr("确认移除项目类别记录？项目文件夹及其中的专用 Skills 均保留。") : tr("确认删除这个虚拟类别？此操作只删除类别及成员归属，不会删除任何 Skill、向量数据或 Agent 安装。")}</p>
            {categoryDeleteDialog.error && (<p className="skill-action-dialog__error" role="alert">
                {categoryDeleteDialog.error}
              </p>)}
            <footer>
              <button type="button" disabled={categoryDeleteDialog.busy} onClick={() => setCategoryDeleteDialog(undefined)}>{tr("取消")}</button>
              <button type="button" className="danger-button" disabled={categoryDeleteDialog.busy} onClick={deleteCustomCategory}>
                {categoryDeleteDialog.busy ? tr("正在删除…") : tr("确认删除")}
              </button>
            </footer>
          </section>
        </div>)}

      {agentWindowDeleteDialog !== undefined && (<div className="skill-action-backdrop" role="presentation">
          <section className="skill-action-dialog category-delete-dialog" role="alertdialog" aria-modal="true" aria-labelledby="agent-window-delete-title">
            <header>
              <div>
                <p className="eyebrow">{tr("Agent 窗口")}</p>
                <h3 id="agent-window-delete-title">{tr("删除 Agent 窗口")}</h3>
              </div>
              <button type="button" className="modal-close" aria-label={tr("关闭删除 Agent 窗口确认")} onClick={() => setAgentWindowDeleteDialog(undefined)}>
                ×
              </button>
            </header>
            <strong>Agent {agentWindowDeleteDialog}</strong>
            <p>{tr("确认删除这个窗口？该窗口当前会话中的未发送输入会丢失；不会删除其他 Agent 窗口、Skills、API Keys 或向量数据。")}</p>
            <footer>
              <button type="button" onClick={() => setAgentWindowDeleteDialog(undefined)}>{tr("取消")}</button>
              <button type="button" className="danger-button" onClick={deleteAgentWindow}>{tr("确认删除")}</button>
            </footer>
          </section>
        </div>)}

      <main className="main-platform" aria-label={tr("A1 可视化工作区")}>
            {activeView && (<SkillGraph activeView={activeView} viewLabel={activeCustomCategory?.name} isNative={isNativeRuntime} refreshKey={graphRevision + libraryVersion} highlightSkillIds={activeAgentWindow !== null && agentHighlights[activeAgentWindow]?.viewId === activeView ? agentHighlights[activeAgentWindow].skillIds : []}/>)}
      </main>

      <aside className="assistant-platform" aria-label={tr("B3 Agent 平台")}>
        <div className="assistant-platform__content" aria-hidden={!isAssistantOpen}>
          {agentWindows.ids.map((windowId) => (<div key={windowId} className="assistant-platform__pane" hidden={activeAgentWindow !== windowId} aria-label={tr("Agent {0} 窗口", windowId)}>
                <AgentComposer windowId={windowId} indexRevision={graphRevision + libraryVersion} activeView={agentWindows.bindings[String(windowId)]} skills={skillResult?.skills ?? []} categories={customCategories} onCreateCategory={createCustomCategory} onCopy={copyRecommendedSkills} onRecommendationChange={(id, viewId, skillIds) => setAgentHighlights((current) => { const next = { ...current }; if (viewId)
            next[id] = { viewId, skillIds };
        else
            delete next[id]; return next; })}/>
              </div>))}
        </div>
        <nav className="assistant-platform__rail" aria-label={tr("Agent 窗口列表")}>
          {agentWindows.ids.map((windowId) => {
            const selected = activeAgentWindow === windowId;
            const boundView = agentWindows.bindings[String(windowId)];
            const boundLabel = viewLabel(boundView);
            const boundCategory = customCategories.find((category) => `custom:${category.categoryId}` === boundView);
            const label = windowId === 1
                ? selected ? tr("收起 Agent 1") : tr("打开 Agent 1")
                : selected ? tr("收起 Agent {0}", windowId) : tr("打开 Agent {0}", windowId);
            return (<button key={windowId} className={`agent-button category-button assistant-entry ${selected ? "agent-button--active" : ""}`} type="button" aria-label={label} aria-pressed={selected} aria-expanded={selected} title={`Agent ${windowId} · ${boundLabel}`} onClick={() => toggleAgentWindow(windowId)} onContextMenu={(event) => {
                    event.preventDefault();
                    setAgentWindowContextMenu({
                        windowId,
                        x: Math.max(8, Math.min(event.clientX, window.innerWidth - 176)),
                        y: Math.max(8, Math.min(event.clientY, window.innerHeight - 64)),
                    });
                }}>
                  {boundCategory?.color && <span className="category-button__dot" style={{ backgroundColor: boundCategory.color }} aria-hidden="true"/>}
                  <span className="assistant-entry__label">Agent {windowId}</span>
                </button>);
        })}
          <button className="agent-button agent-button--add assistant-entry" type="button" aria-label={tr("新建 Agent 窗口")} title={activeView ? tr("为“{0}”新建 Agent", viewLabel(activeView)) : tr("请先选择一个 Skills 类别")} disabled={!activeView || agentWindows.ids.length >= 100} onClick={createAgentWindow}>
            <span aria-hidden="true">＋</span><span className="assistant-entry__label">{tr("新建")}</span>
          </button>
          <p className="assistant-platform__caption">{tr("自动整理skills agent")}</p>
        </nav>
      </aside>

      {isKeyDialogOpen && (<SettingsDialog isNative={isNativeRuntime} keyInputRef={keyInputRef} onClose={closeKeyDialog}/>)}
      {(isCreateCategoryOpen || isCreateProjectOpen) && (<CreateCategoryDialog project={isCreateProjectOpen} skills={skillResult?.skills ?? []} onClose={() => { setIsCreateCategoryOpen(false); setIsCreateProjectOpen(false); }} onCreate={createCustomCategory}/>)}
    </div>);
}
function NavigationGroupToggle({ id, label, shortLabel, expanded, onToggle, children, }: {
    id: NavigationGroup;
    label: string;
    shortLabel: string;
    expanded: boolean;
    onToggle: () => void;
    children: ReactNode;
}) {
    const contentId = `navigation-group-${id}`;
    return (<section className="navigation-group">
      <button className="navigation-group__toggle" type="button" aria-label={label} aria-expanded={expanded} aria-controls={contentId} onClick={onToggle}>
        <span className="navigation-group__label">{label}</span>
        <span className="navigation-group__short">{shortLabel}</span>
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d={expanded ? "M4 6l4 4 4-4" : "M6 4l4 4-4 4"} fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
        </svg>
      </button>
      {expanded && (<div id={contentId} className="navigation-group__content">
          {children}
        </div>)}
    </section>);
}
export default App;
