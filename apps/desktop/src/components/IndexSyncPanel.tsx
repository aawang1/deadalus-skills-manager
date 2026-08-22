import { useEffect, useState } from "react";
import { api, isUnavailableCommandError } from "../api";
import type {
  EmbeddingJob,
  EmbeddingProfile,
  IndexDiff,
  IndexSyncStatus,
  PreflightEstimate,
} from "../types";

interface IndexSyncPanelProps {
  isNative: boolean;
  activeProfileId?: string;
  profiles: EmbeddingProfile[];
  onEstimate: (
    estimate: PreflightEstimate,
    execution?: {
      strategy: "full_rebuild" | "profile_migration";
      profileId?: string;
    },
  ) => void;
  refreshVersion: number;
  notify: (type: "info" | "success" | "error", message: string) => void;
}

export function IndexSyncPanel({
  isNative,
  activeProfileId,
  profiles,
  onEstimate,
  refreshVersion,
  notify,
}: IndexSyncPanelProps) {
  const [status, setStatus] = useState<IndexSyncStatus>();
  const [diff, setDiff] = useState<IndexDiff>();
  const [jobs, setJobs] = useState<EmbeddingJob[]>([]);
  const [busy, setBusy] = useState<string>();
  const [workerAvailable, setWorkerAvailable] = useState(true);
  const [targetProfileId, setTargetProfileId] = useState(
    activeProfileId ?? profiles[0]?.profileId ?? "",
  );

  useEffect(() => {
    if (
      targetProfileId &&
      profiles.some((profile) => profile.profileId === targetProfileId)
    ) {
      return;
    }
    setTargetProfileId(activeProfileId ?? profiles[0]?.profileId ?? "");
  }, [activeProfileId, profiles, targetProfileId]);

  const unavailable = (error: unknown) => {
    if (isUnavailableCommandError(error)) {
      setWorkerAvailable(false);
      notify("info", "索引 Worker 尚未在当前后端启用；界面保持只读。");
      return;
    }
    notify("error", String(error));
  };

  useEffect(() => {
    if (!isNative) return;
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
    if (!status) return;
    setBusy("auto");
    try {
      setStatus(await api.setIndexAutoUpdate(!status.autoUpdate));
      notify("success", "自动更新设置已保存。");
    } catch (error) {
      unavailable(error);
    } finally {
      setBusy(undefined);
    }
  };

  const scan = async () => {
    setBusy("scan");
    try {
      setDiff(await api.scanEmbeddingChanges());
      notify("success", "变更扫描完成。");
    } catch (error) {
      unavailable(error);
    } finally {
      setBusy(undefined);
    }
  };

  const rebuild = async () => {
    if (!targetProfileId) {
      notify(
        "info",
        "请先在“Embedding Profiles”页创建一个 Profile，再开始全量扫描。",
      );
      return;
    }
    setBusy("rebuild");
    try {
      onEstimate(await api.beginFullRebuildPreflight(targetProfileId), {
        strategy: "full_rebuild",
        profileId: targetProfileId,
      });
    } catch (error) {
      unavailable(error);
    } finally {
      setBusy(undefined);
    }
  };

  const applyIncremental = async () => {
    if (!activeProfileId || !diff) return;
    if (
      !window.confirm(
        `确认应用增量更新吗？新增 ${diff.added}、修改 ${diff.changed}、删除 ${diff.removed}。`,
      )
    ) {
      return;
    }
    setBusy("incremental");
    try {
      const job = await api.startEmbeddingJob(
        "incremental",
        undefined,
        activeProfileId,
      );
      setJobs((current) => [job, ...current]);
      notify("success", "增量更新任务已启动。");
    } catch (error) {
      unavailable(error);
    } finally {
      setBusy(undefined);
    }
  };

  const cancelJob = async (jobId: string) => {
    try {
      await api.cancelEmbeddingJob(jobId);
      setJobs(await api.listEmbeddingJobs());
      notify("info", "已请求取消任务。");
    } catch (error) {
      unavailable(error);
    }
  };

  const disabled = !isNative || !workerAvailable;

  return (
    <div className="settings-panel index-panel">
      {(!isNative || !workerAvailable) && (
        <div className="inline-notice">
          {!isNative
            ? "浏览器预览不会启动本机扫描或索引任务。"
            : "当前后端尚未提供索引 Worker 命令；控件已安全禁用。"}
        </div>
      )}

      <section className="panel-section sync-overview">
        <header className="section-heading">
          <div><h3>索引状态</h3><p>Canonical Snapshot → Embedding Index</p></div>
          <span className={`status-badge ${status ? "status-badge--active" : ""}`}>
            {status ? "CONNECTED" : "UNAVAILABLE"}
          </span>
        </header>
        <div className="metric-grid">
          <div><span>已索引 Skills</span><strong>{status?.indexedSkills ?? "—"}</strong></div>
          <div><span>待同步变更</span><strong>{status?.pendingChanges ?? "—"}</strong></div>
          <div><span>活动 Profile</span><strong>{activeProfileId ? "READY" : "NONE"}</strong></div>
          <div><span>最近同步</span><strong>{status?.lastSyncedAt ? new Date(status.lastSyncedAt * 1000).toLocaleString() : "—"}</strong></div>
        </div>
      </section>

      <section className="panel-section sync-controls">
        <div className="toggle-row">
          <div><h3>自动更新</h3><p>统一快照变更后安排增量索引。</p></div>
          <button
            className={`toggle ${status?.autoUpdate ? "is-on" : ""}`}
            type="button"
            role="switch"
            aria-checked={status?.autoUpdate ?? false}
            disabled={disabled || !status || busy === "auto"}
            onClick={toggleAutoUpdate}
          >
            <span />
          </button>
        </div>
        <label className="sync-profile-target">
          <span>全量扫描目标 Profile</span>
          <select
            value={targetProfileId}
            disabled={disabled || busy != null || profiles.length === 0}
            onChange={(event) => setTargetProfileId(event.currentTarget.value)}
          >
            {profiles.length === 0 ? (
              <option value="">尚未创建 Profile</option>
            ) : (
              profiles.map((profile) => (
                <option key={profile.profileId} value={profile.profileId}>
                  {profile.provider} / {profile.model} / {profile.status}
                </option>
              ))
            )}
          </select>
        </label>
        <div className="sync-buttons">
          <button
            type="button"
            disabled={disabled || busy != null}
            onClick={scan}
          >
            {busy === "scan" ? "扫描中…" : "扫描变更"}
          </button>
          <button
            type="button"
            disabled={
              disabled ||
              busy != null ||
              !activeProfileId ||
              !diff ||
              diff.added + diff.changed + diff.removed === 0
            }
            onClick={applyIncremental}
          >
            {busy === "incremental" ? "启动中…" : "应用增量"}
          </button>
          <button
            className="danger-outline"
            type="button"
            disabled={disabled || busy != null}
            onClick={rebuild}
          >
            {busy === "rebuild" ? "估算中…" : "全量重建"}
          </button>
        </div>
      </section>

      <div className="index-detail-grid">
        <section className="panel-section">
          <header className="section-heading"><div><h3>Diff</h3><p>上次扫描结果</p></div></header>
          <dl className="diff-list">
            <div><dt>新增</dt><dd>{diff?.added ?? "—"}</dd></div>
            <div><dt>修改</dt><dd>{diff?.changed ?? "—"}</dd></div>
            <div><dt>删除</dt><dd>{diff?.removed ?? "—"}</dd></div>
            <div><dt>未变</dt><dd>{diff?.unchanged ?? "—"}</dd></div>
          </dl>
        </section>
        <section className="panel-section">
          <header className="section-heading"><div><h3>Jobs</h3><p>最近向量任务</p></div><span>{jobs.length}</span></header>
          <div className="job-list">
            {jobs.length === 0 ? <div className="empty-card">暂无任务</div> : jobs.map((job) => (
              <div key={job.jobId}>
                <span>{job.kind}</span>
                <strong>{job.status}</strong>
                <progress value={job.completedItems} max={job.totalItems || 1} />
                {["pending", "running", "paused"].includes(job.status) && (
                  <button type="button" onClick={() => cancelJob(job.jobId)}>
                    取消
                  </button>
                )}
              </div>
            ))}
          </div>
        </section>
      </div>
    </div>
  );
}
