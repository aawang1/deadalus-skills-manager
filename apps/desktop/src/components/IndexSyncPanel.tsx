import { tr, displayStatus, getLanguage } from "../i18n";
import { useEffect, useState } from "react";
import { api, isUnavailableCommandError } from "../api";
import type { EmbeddingJob, EmbeddingProfile, IndexDiff, IndexSyncStatus, PreflightEstimate, } from "../types";
interface IndexSyncPanelProps {
    isNative: boolean;
    activeProfileId?: string;
    profiles: EmbeddingProfile[];
    onEstimate: (estimate: PreflightEstimate, execution?: {
        strategy: "incremental" | "full_rebuild" | "profile_migration";
        profileId?: string;
    }) => void;
    refreshVersion: number;
    notify: (type: "info" | "success" | "error", message: string) => void;
}
export function IndexSyncPanel({ isNative, activeProfileId, profiles, onEstimate, refreshVersion, notify, }: IndexSyncPanelProps) {
    const [status, setStatus] = useState<IndexSyncStatus>();
    const [diff, setDiff] = useState<IndexDiff>();
    const [jobs, setJobs] = useState<EmbeddingJob[]>([]);
    const [busy, setBusy] = useState<string>();
    const [workerAvailable, setWorkerAvailable] = useState(true);
    const [targetProfileId, setTargetProfileId] = useState(activeProfileId ?? profiles[0]?.profileId ?? "");
    useEffect(() => {
        if (targetProfileId &&
            profiles.some((profile) => profile.profileId === targetProfileId)) {
            return;
        }
        setTargetProfileId(activeProfileId ?? profiles[0]?.profileId ?? "");
    }, [activeProfileId, profiles, targetProfileId]);
    const unavailable = (error: unknown) => {
        if (isUnavailableCommandError(error)) {
            setWorkerAvailable(false);
            notify("info", tr("索引 Worker 尚未在当前后端启用；界面保持只读。"));
            return;
        }
        notify("error", String(error));
    };
    useEffect(() => {
        if (!isNative)
            return;
        Promise.all([
            api.getIndexSyncStatus(),
            api.getIndexDiff(),
            api.listEmbeddingJobs(),
        ])
            .then(([nextStatus, nextDiff, nextJobs]) => {
            setStatus(nextStatus);
            setDiff(nextDiff);
            setJobs(nextJobs);
        })
            .catch(unavailable);
    }, [isNative, refreshVersion]);
    const toggleAutoUpdate = async () => {
        if (!status)
            return;
        setBusy("auto");
        try {
            setStatus(await api.setIndexAutoUpdate(!status.autoUpdate));
            notify("success", tr("自动更新设置已保存。"));
        }
        catch (error) {
            unavailable(error);
        }
        finally {
            setBusy(undefined);
        }
    };
    const scan = async () => {
        setBusy("scan");
        try {
            setDiff(await api.scanEmbeddingChanges());
            notify("success", tr("变更扫描完成。"));
        }
        catch (error) {
            unavailable(error);
        }
        finally {
            setBusy(undefined);
        }
    };
    const rebuild = async () => {
        if (!targetProfileId) {
            notify("info", tr("请先在“Embedding Profiles”页创建一个 Profile，再开始全量扫描。"));
            return;
        }
        setBusy("rebuild");
        try {
            onEstimate(await api.beginFullRebuildPreflight(targetProfileId), {
                strategy: "full_rebuild",
                profileId: targetProfileId,
            });
        }
        catch (error) {
            unavailable(error);
        }
        finally {
            setBusy(undefined);
        }
    };
    const applyIncremental = async () => {
        if (!activeProfileId || !diff)
            return;
        setBusy("incremental");
        try {
            onEstimate(await api.beginIncrementalPreflight(activeProfileId), {
                strategy: "incremental",
                profileId: activeProfileId,
            });
        }
        catch (error) {
            unavailable(error);
        }
        finally {
            setBusy(undefined);
        }
    };
    const cancelJob = async (jobId: string) => {
        try {
            await api.cancelEmbeddingJob(jobId);
            setJobs(await api.listEmbeddingJobs());
            notify("info", tr("已请求取消任务。"));
        }
        catch (error) {
            unavailable(error);
        }
    };
    const changeComplexity = async (level: 1 | 2 | 3 | 4) => {
        if (!status || level === status.vectorizationComplexity)
            return;
        setBusy("complexity");
        try {
            const next = await api.setVectorizationComplexity(level);
            setStatus(next);
            setDiff(await api.getIndexDiff());
            notify("success", tr("处理深度已切换为 {0}；后续增量与全量任务使用此档位。", complexityLevels[level - 1].label));
        }
        catch (error) {
            unavailable(error);
        }
        finally {
            setBusy(undefined);
        }
    };
    const toggleBuiltIns = async () => {
        if (!status)
            return;
        setBusy("built-ins");
        try {
            const next = await api.setIgnoreBuiltInSkills(!status.ignoreBuiltInSkills);
            setStatus(next);
            setDiff(await api.scanEmbeddingChanges());
            notify("success", next.ignoreBuiltInSkills
                ? tr("后续分析、向量化与图表将忽略内置 Skills。") : tr("内置 Skills 已重新纳入索引范围。"));
        }
        catch (error) {
            unavailable(error);
        }
        finally {
            setBusy(undefined);
        }
    };
    const deleteJobHistory = async (job: EmbeddingJob) => {
        if (!window.confirm(tr("确定删除这条 {0} / {1} Job 历史吗？此操作无法撤销。", job.kind, job.status))) {
            return;
        }
        setBusy(`delete:${job.jobId}`);
        try {
            const deleted = await api.deleteEmbeddingJobHistory(job.jobId);
            if (!deleted) {
                notify("info", tr("该 Job 仍在活动中，未删除。"));
                return;
            }
            setJobs((current) => current.filter((item) => item.jobId !== job.jobId));
            notify("success", tr("Job 历史已删除。"));
        }
        catch (error) {
            unavailable(error);
        }
        finally {
            setBusy(undefined);
        }
    };
    const clearJobHistory = async () => {
        const finishedCount = jobs.filter((job) => isFinishedJob(job.status)).length;
        if (finishedCount === 0)
            return;
        if (!window.confirm(tr("确定清空 {0} 条已结束的 Jobs 历史吗？活动任务会保留，此操作无法撤销。", finishedCount))) {
            return;
        }
        setBusy("clear-jobs");
        try {
            const deleted = await api.clearEmbeddingJobHistory();
            setJobs(await api.listEmbeddingJobs());
            notify("success", tr("已删除 {0} 条 Jobs 历史。", deleted));
        }
        catch (error) {
            unavailable(error);
        }
        finally {
            setBusy(undefined);
        }
    };
    const disabled = !isNative || !workerAvailable;
    return (<div className="settings-panel index-panel">
      {(!isNative || !workerAvailable) && (<div className="inline-notice">
          {!isNative
                ? tr("浏览器预览不会启动本机扫描或索引任务。") : tr("当前后端尚未提供索引 Worker 命令；控件已安全禁用。")}
        </div>)}

      <section className="panel-section sync-overview">
        <header className="section-heading">
          <div><h3>{tr("索引状态")}</h3><p>{tr("规范快照 → 向量索引")}</p></div>
          <span className={`status-badge ${status ? "status-badge--active" : ""}`}>
            {status ? "CONNECTED" : "UNAVAILABLE"}
          </span>
        </header>
        <div className="metric-grid">
          <div><span>{tr("已索引 Skills")}</span><strong>{status?.indexedSkills ?? "—"}</strong></div>
          <div><span>{tr("待同步变更")}</span><strong>{status?.pendingChanges ?? "—"}</strong></div>
          <div><span>{tr("活动 Profile")}</span><strong>{activeProfileId ? "READY" : "NONE"}</strong></div>
          <div><span>{tr("最近同步")}</span><strong>{status?.lastSyncedAt ? new Date(status.lastSyncedAt * 1000).toLocaleString(getLanguage() === "zh" ? "zh-CN" : "en-US") : "—"}</strong></div>
        </div>
      </section>

      <section className="panel-section sync-controls">
        <div className="toggle-row">
          <div><h3>{tr("自动更新")}</h3><p>{tr("统一快照变更后安排增量索引。")}</p></div>
          <button className={`toggle ${status?.autoUpdate ? "is-on" : ""}`} type="button" role="switch" aria-checked={status?.autoUpdate ?? false} disabled={disabled || !status || busy === "auto"} onClick={toggleAutoUpdate}>
            <span />
          </button>
        </div>
        <label className="sync-profile-target">
          <span>{tr("全量扫描目标 Profile")}</span>
          <select value={targetProfileId} disabled={disabled || busy != null || profiles.length === 0} onChange={(event) => setTargetProfileId(event.currentTarget.value)}>
            {profiles.length === 0 ? (<option value="">{tr("尚未创建 Profile")}</option>) : (profiles.map((profile) => (<option key={profile.profileId} value={profile.profileId}>
                  {profile.name || profile.model} / {profile.provider} / {displayStatus(profile.status)}
                </option>)))}
          </select>
        </label>
        <div className="sync-buttons">
          <label className="compact-switch" title={tr("同时影响分析、向量化、搜索和图表显示")}>
            <button className={`toggle toggle--compact ${status?.ignoreBuiltInSkills ? "is-on" : ""}`} type="button" role="switch" aria-checked={status?.ignoreBuiltInSkills ?? false} disabled={disabled || !status || busy != null} onClick={toggleBuiltIns}>
              <span />
            </button>
            <span>{tr("忽略内置 Skills")}</span>
          </label>
          <div className="complexity-control">
            <div className="complexity-control__label">
              <span>{tr("处理深度")}</span>
              <strong>
                {complexityLevels[(status?.vectorizationComplexity ?? 4) - 1].label}
              </strong>
            </div>
            <input className={`complexity-control__range complexity-control__range--${status?.vectorizationComplexity ?? 4}`} type="range" min="1" max="4" step="1" value={status?.vectorizationComplexity ?? 4} disabled={disabled || !status || busy != null} aria-label={tr("向量化处理深度")} aria-valuetext={complexityLevels[(status?.vectorizationComplexity ?? 4) - 1].label} onChange={(event) => changeComplexity(Number(event.currentTarget.value) as 1 | 2 | 3 | 4)}/>
            <div className="complexity-control__marks" aria-hidden="true">
              {complexityLevels.map((item) => (<span key={item.level} data-tooltip={item.detail}>
                  {item.level}
                </span>))}
            </div>
          </div>
          <button type="button" disabled={disabled || busy != null} onClick={scan}>
            {busy === "scan" ? tr("扫描中…") : tr("扫描变更")}
          </button>
          <button type="button" disabled={disabled ||
            busy != null ||
            !activeProfileId ||
            !diff ||
            diff.added + diff.changed + diff.removed === 0} onClick={applyIncremental}>
            {busy === "incremental" ? tr("启动中…") : tr("应用增量")}
          </button>
          <button className="danger-outline" type="button" disabled={disabled || busy != null} onClick={rebuild}>
            {busy === "rebuild" ? tr("估算中…") : tr("全量重建")}
          </button>
        </div>
      </section>

      <div className="index-detail-grid">
        <section className="panel-section">
          <header className="section-heading"><div><h3>{tr("差异")}</h3><p>{tr("上次扫描结果")}</p></div></header>
          <dl className="diff-list">
            <div><dt>{tr("新增")}</dt><dd>{diff?.added ?? "—"}</dd></div>
            <div><dt>{tr("修改")}</dt><dd>{diff?.changed ?? "—"}</dd></div>
            <div><dt>{tr("删除")}</dt><dd>{diff?.removed ?? "—"}</dd></div>
            <div><dt>{tr("未变")}</dt><dd>{diff?.unchanged ?? "—"}</dd></div>
          </dl>
        </section>
        <section className="panel-section">
          <header className="section-heading"><div><h3>{tr("任务")}</h3><p>{tr("最近向量任务")}</p></div><div className="job-heading-actions"><span>{jobs.length}</span><button className="danger-outline" type="button" disabled={disabled || busy != null || !jobs.some((job) => isFinishedJob(job.status))} onClick={clearJobHistory}>{busy === "clear-jobs" ? tr("清理中…") : tr("清空历史")}</button></div></header>
          <div className="job-list">
            {jobs.length === 0 ? <div className="empty-card">{tr("暂无任务")}</div> : jobs.map((job) => (<div key={job.jobId}>
                <span>{displayStatus(job.kind)}</span>
                <strong>{displayStatus(job.status)}</strong>
                <progress value={job.completedItems} max={job.totalItems || 1}/>
                {["pending", "running", "paused"].includes(job.status) && (<button type="button" onClick={() => cancelJob(job.jobId)}>{tr("取消")}</button>)}
                {isFinishedJob(job.status) && (<button className="job-delete-button" type="button" disabled={disabled || busy != null} aria-label={tr("删除 {0} Job 历史", job.kind)} onClick={() => deleteJobHistory(job)}>
                    {busy === `delete:${job.jobId}` ? tr("删除中…") : tr("删除历史")}
                  </button>)}
              </div>))}
          </div>
        </section>
      </div>
    </div>);
}
const complexityLevels = [
    { level: 1 as const, get label() {
            return tr("核心");
        }, get detail() {
            return tr("LLM、结构分析与 Embedding 仅读取核心 SKILL.md；Overall Function 使用规则生成内容。");
        } },
    { level: 2 as const, get label() {
            return tr("摘要");
        }, get detail() {
            return tr("扫描范围仍仅为 SKILL.md，并使用最终 LLM 分类摘要重新生成 Overall Function 向量。");
        } },
    { level: 3 as const, get label() {
            return tr("全量向量");
        }, get detail() {
            return tr("LLM 与结构分析仅读取 SKILL.md；Embedding 扩展到 Skill 包内全部可嵌入文件。");
        } },
    { level: 4 as const, get label() {
            return tr("完整");
        }, get detail() {
            return tr("LLM、结构分析与 Embedding 均读取 Skill 包内全部可嵌入文件，耗时与 Token 最高。");
        } },
];
function isFinishedJob(status: EmbeddingJob["status"]) {
    return ["completed", "failed", "cancelled"].includes(status);
}
