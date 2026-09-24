import { DisplaySummary } from "./DisplaySummary";
import { tr } from "../i18n";
import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { AgentSkillRecommendation, CreateCustomSkillCategoryRequest, CustomSkillCategory, InstalledSkill, ViewId } from "../types";
import { CreateCategoryDialog } from "./CreateCategoryDialog";
interface Props {
    windowId: number;
    indexRevision?: number;
    activeView?: ViewId | null;
    skills?: InstalledSkill[];
    categories?: CustomSkillCategory[];
    onCreateCategory?: (request: CreateCustomSkillCategoryRequest) => Promise<void>;
    onCopy?: (skillIds: string[], target: string, mode: "incremental" | "overwrite") => Promise<boolean>;
    onRecommendationChange?: (windowId: number, viewId: ViewId | null, skillIds: string[]) => void;
}
export function AgentComposer({ windowId, indexRevision = 0, activeView = null, skills = [], categories = [], onCreateCategory = async () => { }, onCopy = async () => false, onRecommendationChange = () => { } }: Props) {
    const [draft, setDraft] = useState("");
    const [creating, setCreating] = useState(false);
    const [hint, setHint] = useState(false);
    const [recommendation, setRecommendation] = useState<AgentSkillRecommendation>();
    const [pending, setPending] = useState(false);
    const [error, setError] = useState("");
    const [notice, setNotice] = useState("");
    const [confirmDelete, setConfirmDelete] = useState(false);
    const [createOpen, setCreateOpen] = useState(false);
    const [copyMode, setCopyMode] = useState<"incremental" | "overwrite" | null>(null);
    const [copyTarget, setCopyTarget] = useState("");
    const hintTimer = useRef<number | undefined>(undefined);
    const requestId = useRef(0);
    useEffect(() => () => window.clearTimeout(hintTimer.current), []);
    useEffect(() => {
        requestId.current += 1;
        setRecommendation(undefined);
        setError("");
        setCreateOpen(false);
        setCopyMode(null);
        setConfirmDelete(false);
        onRecommendationChange(windowId, null, []);
        // A result belongs to exactly one visualized category.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [activeView, windowId, indexRevision]);
    const send = async () => {
        if (pending)
            return;
        if (!creating) {
            setError(tr("请先点击“新建”，让当前 Agent 进入需求分析状态。"));
            return;
        }
        if (!activeView) {
            setError(tr("请先在左侧选择一个 Skills 类别；推荐范围只限于当前可视化类别。"));
            return;
        }
        const request = draft.trim();
        if (!request) {
            setError(tr("请输入需要的 Skills 类型、功能、数据类型或希望完成的工作。"));
            return;
        }
        const id = ++requestId.current;
        setPending(true);
        setError("");
        setNotice("");
        try {
            const result = await api.recommendAgentSkills(activeView, request);
            if (id !== requestId.current)
                return;
            setRecommendation(result);
            onRecommendationChange(windowId, activeView, result.results.map((item) => item.skillId));
            if (!result.results.length)
                setNotice(result.warnings?.length ? tr("部分候选分析失败，且未得到可展示结果。") : tr("当前类别没有达到匹配门槛的 Skills。可调整需求后重试。"));
        }
        catch (nextError) {
            if (id === requestId.current)
                setError(String(nextError));
        }
        finally {
            if (id === requestId.current)
                setPending(false);
        }
    };
    const clear = () => { setRecommendation(undefined); setConfirmDelete(false); setNotice(""); onRecommendationChange(windowId, null, []); };
    const ids = recommendation?.results.map((item) => item.skillId) ?? [];
    const byId = new Map(skills.map((skill) => [skill.skillId, skill]));
    const copyable = ids.filter((id) => !byId.get(id)?.isBuiltIn);
    return <div className="agent-workspace" aria-label={tr("Agent {0} 输入区", windowId)}>
    {recommendation && <section className="agent-recommendations" aria-label={tr("推荐 Skills")}>
      <header><div><strong>{tr("匹配的 Skills ·")}{ids.length}</strong><small>{recommendation.summary}</small></div><button className="agent-recommendations__delete" type="button" aria-label={tr("删除推荐结果")} onClick={() => setConfirmDelete(true)}>×</button></header>
      {!!recommendation.warnings?.length && <div className="agent-recommendations__warnings" role="status">{recommendation.warnings.map((warning) => <small key={warning}>{warning}</small>)}</div>}
      <div className="agent-recommendations__list">{recommendation.results.map((item) => { const skill = byId.get(item.skillId); return <article key={item.skillId}><strong>{skill?.name ?? item.skillId}</strong><span>{Math.round(item.score * 100)}%</span><p><DisplaySummary text={skill?.description}/></p>{skill?.isBuiltIn && <small>{tr("内置 Skill · 不参与复制")}</small>}</article>; })}</div>
      <footer><button type="button" disabled={!copyable.length} onClick={() => setCreateOpen(true)}>{tr("创建新类别")}</button><button type="button" disabled={!copyable.length} onClick={() => { setCopyMode("incremental"); setCopyTarget(""); }}>{tr("增量复制")}</button><button type="button" disabled={!copyable.length} onClick={() => { setCopyMode("overwrite"); setCopyTarget(""); }}>{tr("全量复制")}</button></footer>
    </section>}
    {notice && <p role="status">{notice}</p>}{error && <p role="alert" className="agent-workspace__error">{error}</p>}
    <form className="agent-composer" onSubmit={(event) => { event.preventDefault(); void send(); }}>
      <span className="agent-composer__new-wrap" onMouseEnter={() => { window.clearTimeout(hintTimer.current); hintTimer.current = window.setTimeout(() => setHint(true), 1000); }} onMouseLeave={() => { window.clearTimeout(hintTimer.current); setHint(false); }}><button className={`agent-composer__new${creating ? " agent-composer__new--active" : ""}`} type="button" aria-pressed={creating} onClick={() => { setCreating((value) => !value); setError(""); }}>{tr("新建")}</button>{hint && <span className="agent-composer__hint" role="tooltip">{tr("自动新建skills类别")}</span>}</span>
      <label className="agent-composer__input"><span className="sr-only">{tr("Agent {0} 输入", windowId)}</span><input type="text" value={draft} placeholder={creating ? tr("描述所需 Skills、功能、数据或任务…") : tr("输入内容…")} onChange={(event) => { setDraft(event.currentTarget.value); setError(""); }}/></label>
      <button className="agent-composer__send" type="submit" disabled={pending}>{pending ? tr("分析中") : tr("发送")}</button>
    </form>
    {confirmDelete && <div className="dialog-backdrop"><section className="settings-dialog agent-action-dialog" role="alertdialog" aria-modal="true" aria-label={tr("删除推荐结果")}><h2>{tr("删除这组推荐结果？")}</h2><p>{tr("图中高亮将清除，不会删除 Skill 文件。")}</p><footer><button type="button" onClick={() => setConfirmDelete(false)}>{tr("取消")}</button><button type="button" className="danger-button" onClick={clear}>{tr("确认删除")}</button></footer></section></div>}
    {createOpen && <CreateCategoryDialog skills={skills} fixedSkillIds={copyable} onClose={() => setCreateOpen(false)} onCreate={async (request) => { await onCreateCategory(request); setCreateOpen(false); setNotice(tr("已创建类别。")); }}/>}
    {copyMode && <div className="dialog-backdrop"><section className="settings-dialog agent-action-dialog" role="dialog" aria-modal="true" aria-label={tr("复制推荐 Skills")}><h2>{copyMode === "overwrite" ? tr("全量覆盖目标") : tr("增量复制")}</h2><p>{copyMode === "overwrite" ? tr("目标中多余的 Skill 将被移除；共享安装可能改为禁用。") : tr("只复制目标尚未拥有的 Skill。")}</p><label>{tr("选择目标")}<select value={copyTarget} onChange={(event) => setCopyTarget(event.currentTarget.value)}><option value="">{tr("请选择")}</option>{["claude-code", "cursor", "codex"].map((id) => <option key={id} value={`agent:${id}`}>{id}</option>)}{categories.map((item) => <option key={item.categoryId} value={`custom:${item.categoryId}`}>{item.name}</option>)}</select></label>{error && <p role="alert" className="agent-workspace__error">{error}</p>}<footer><button type="button" disabled={pending} onClick={() => setCopyMode(null)}>{tr("取消")}</button><button type="button" disabled={!copyTarget || pending} onClick={async () => { setPending(true); setError(""); try {
        const applied = await onCopy(copyable, copyTarget, copyMode);
        setCopyMode(null);
        if (applied)
            setNotice(tr("复制操作已提交或完成。"));
    }
    catch (nextError) {
        setError(String(nextError));
    }
    finally {
        setPending(false);
    } }}>{tr("确认")}{copyMode === "overwrite" ? tr("覆盖") : tr("复制")}</button></footer></section></div>}
  </div>;
}
