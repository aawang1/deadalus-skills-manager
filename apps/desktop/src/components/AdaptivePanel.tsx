import { useEffect, useState } from "react";
import { api } from "../api";
import type {
  AdaptiveHistoryRecord,
  AdaptiveProposal,
  EmbeddingProfile,
  EmbeddingProfileSettings,
  ValidationRun,
} from "../types";

interface AdaptivePanelProps {
  isNative: boolean;
  profiles: EmbeddingProfile[];
  notify: (type: "info" | "success" | "error", message: string) => void;
}

function adaptiveProposal(value: unknown): AdaptiveProposal | undefined {
  if (!value || typeof value !== "object" || !("policy" in value)) {
    return undefined;
  }
  const proposal = value as AdaptiveProposal;
  return proposal.policy && typeof proposal.policy.h === "number"
    ? proposal
    : undefined;
}

export function AdaptivePanel({
  isNative,
  profiles,
  notify,
}: AdaptivePanelProps) {
  const initialProfile =
    profiles.find((profile) => profile.isActive) ?? profiles[0];
  const [profileId, setProfileId] = useState(initialProfile?.profileId ?? "");
  const [settings, setSettings] = useState<EmbeddingProfileSettings>();
  const [history, setHistory] = useState<AdaptiveHistoryRecord[]>([]);
  const [validationRuns, setValidationRuns] = useState<ValidationRun[]>([]);
  const [sampleCount, setSampleCount] = useState(0);
  const [validationBusy, setValidationBusy] = useState<
    "generate" | "run" | "refresh"
  >();
  const [adaptiveBusy, setAdaptiveBusy] = useState<string>();
  const [dataBusy, setDataBusy] = useState<"export" | "reset">();
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!profileId && initialProfile) setProfileId(initialProfile.profileId);
  }, [initialProfile?.profileId, profileId]);

  useEffect(() => {
    if (!isNative || !profileId) {
      setSettings(undefined);
      setHistory([]);
      return;
    }
    let current = true;
    setLoading(true);
    Promise.all([
      api.getEmbeddingProfileSettings(profileId),
      api.listEmbeddingAdaptiveHistory(profileId),
      api.listLocalValidationSamples(profileId),
      api.listLocalValidationRuns(profileId),
    ])
      .then(([nextSettings, nextHistory, samples, runs]) => {
        if (!current) return;
        setSettings(nextSettings);
        setHistory(nextHistory);
        setSampleCount(samples.length);
        setValidationRuns(runs);
      })
      .catch((error) => current && notify("error", String(error)))
      .finally(() => current && setLoading(false));
    return () => {
      current = false;
    };
  }, [isNative, profileId]);

  const refreshValidation = async (kind: "refresh" | "generate" = "refresh") => {
    if (!profileId || !isNative) return;
    setValidationBusy(kind);
    try {
      const samples =
        kind === "generate"
          ? await api.generateLocalValidationSamples(profileId)
          : await api.listLocalValidationSamples(profileId);
      const runs = await api.listLocalValidationRuns(profileId);
      setSampleCount(samples.length);
      setValidationRuns(runs);
      if (kind === "generate") notify("success", "本地检查项已更新。");
    } catch (error) {
      notify("error", String(error));
    } finally {
      setValidationBusy(undefined);
    }
  };

  const runValidation = async () => {
    if (!profileId || !isNative) return;
    setValidationBusy("run");
    try {
      const run = await api.runLocalValidation(profileId);
      setValidationRuns((current) => [
        run,
        ...current.filter((item) => item.runId !== run.runId),
      ]);
      notify("info", "本地质量检查已在后台启动，不会阻止 Profile 使用。");
    } catch (error) {
      notify("error", String(error));
    } finally {
      setValidationBusy(undefined);
    }
  };

  const refreshAdaptive = async () => {
    if (!profileId || !isNative) return;
    const [nextSettings, nextHistory] = await Promise.all([
      api.getEmbeddingProfileSettings(profileId),
      api.listEmbeddingAdaptiveHistory(profileId),
    ]);
    setSettings(nextSettings);
    setHistory(nextHistory);
  };

  const toggleAdaptive = async () => {
    if (!settings || !isNative) return;
    setAdaptiveBusy("toggle");
    try {
      await api.setLocalAdaptiveEnabled(
        profileId,
        !settings.adaptivePolicy.localUpdateEnabled,
      );
      await refreshAdaptive();
      notify("success", "本地自适应开关已更新，仅影响之后创建的更新任务。");
    } catch (error) {
      notify("error", String(error));
    } finally {
      setAdaptiveBusy(undefined);
    }
  };

  const proposeAdaptive = async () => {
    if (!profileId || !isNative) return;
    setAdaptiveBusy("propose");
    try {
      await api.proposeAdaptiveSettings(profileId);
      await refreshAdaptive();
      notify("success", "已生成待审核的本地自适应提案。");
    } catch (error) {
      notify("error", String(error));
    } finally {
      setAdaptiveBusy(undefined);
    }
  };

  const decideProposal = async (
    item: AdaptiveHistoryRecord,
    decision: "apply" | "reject" | "revert",
  ) => {
    const prompts = {
      apply: "确认应用该提案吗？它只影响之后的更新；现有向量需要全量重建。",
      reject: "确认拒绝该自适应提案吗？",
      revert: "确认回滚当前生效的自适应设置吗？之后的更新将使用恢复后的设置。",
    };
    if (!window.confirm(prompts[decision])) return;
    setAdaptiveBusy(item.historyId);
    try {
      if (decision === "apply") {
        await api.applyAdaptiveHistory(item.historyId);
      } else if (decision === "reject") {
        await api.rejectAdaptiveHistory(item.historyId);
      } else {
        await api.revertAdaptiveHistory(item.historyId);
      }
      await refreshAdaptive();
      notify(
        "success",
        decision === "apply"
          ? "提案已应用；现有向量未改变。"
          : decision === "reject"
            ? "提案已拒绝。"
            : "自适应设置已回滚。",
      );
    } catch (error) {
      notify("error", String(error));
    } finally {
      setAdaptiveBusy(undefined);
    }
  };

  const exportValidation = async () => {
    if (!profileId || !isNative) return;
    setDataBusy("export");
    try {
      const value = await api.exportLocalValidationData(profileId);
      const blob = new Blob([JSON.stringify(value, null, 2)], {
        type: "application/json",
      });
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = `deadalus-local-check-${profileId}-${Date.now()}.json`;
      link.click();
      URL.revokeObjectURL(url);
      notify("success", "本地质量检查数据已导出为 JSON。");
    } catch (error) {
      notify("error", String(error));
    } finally {
      setDataBusy(undefined);
    }
  };

  const resetValidation = async () => {
    if (!profileId || !isNative) return;
    const confirmation = window.prompt(
      "这会永久删除当前 Profile 的本地检查项、运行记录和反馈。输入 RESET 继续。",
    );
    if (confirmation !== "RESET") {
      if (confirmation !== null) notify("info", "输入不匹配，未重置任何数据。");
      return;
    }
    setDataBusy("reset");
    try {
      const deleted = await api.resetLocalValidationData(profileId);
      setSampleCount(0);
      setValidationRuns([]);
      notify(
        "success",
        `已重置：${deleted.samples} 项检查、${deleted.runs} 次运行、${deleted.feedbackEvents} 条反馈。`,
      );
    } catch (error) {
      notify("error", String(error));
    } finally {
      setDataBusy(undefined);
    }
  };

  return (
    <div className="settings-panel adaptive-panel">
      {!isNative && (
        <div className="inline-notice">
          浏览器预览无法读取本地 Profile 设置与历史。
        </div>
      )}
      <section className="panel-section">
        <header className="section-heading">
          <div><h3>自适应配置</h3><p>审核本地信号生成的版本化设置提案。</p></div>
          <label className="compact-select">
            <span className="sr-only">选择 Profile</span>
            <select
              value={profileId}
              disabled={!isNative || profiles.length === 0}
              onChange={(event) => setProfileId(event.currentTarget.value)}
            >
              {profiles.map((profile) => (
                <option key={profile.profileId} value={profile.profileId}>
                  {profile.model} · {profile.status}
                </option>
              ))}
            </select>
          </label>
        </header>
        {loading ? (
          <div className="loading-row"><span className="spinner" />读取配置…</div>
        ) : settings ? (
          <>
            <div className="adaptive-bounds" aria-label="自适应边界">
              {(["h", "m", "u", "l"] as const).map((key) => (
                <div key={key}>
                  <span>{key.toUpperCase()}</span>
                  <strong>{settings.adaptivePolicy[key].toLocaleString()}</strong>
                </div>
              ))}
            </div>
            <dl className="policy-meta">
              <div><dt>策略版本</dt><dd>{settings.adaptivePolicy.policyVersion}</dd></div>
              <div>
                <dt>上限来源</dt>
                <dd>
                  {settings.adaptivePolicy.limitSource} ·{" "}
                  {settings.adaptivePolicy.limitSourceVersion}
                </dd>
              </div>
              <div>
                <dt>生效时间</dt>
                <dd>
                  {new Date(
                    settings.adaptivePolicy.effectiveAfter * 1000,
                  ).toLocaleString()}
                </dd>
              </div>
            </dl>
            <div className="inline-notice adaptive-impact-notice">
              自适应变化只影响之后创建的更新任务，不会改写现有向量。
              若要让所有现有向量采用新设置，需要执行全量重建。
            </div>
            <div className="toggle-row adaptive-toggle-row">
              <div>
                <h3>允许本地自适应更新</h3>
                <p>提案仍需明确审核，不会自动应用。</p>
              </div>
              <button
                className={`toggle ${
                  settings.adaptivePolicy.localUpdateEnabled ? "is-on" : ""
                }`}
                type="button"
                role="switch"
                aria-label="允许本地自适应更新"
                aria-checked={settings.adaptivePolicy.localUpdateEnabled}
                disabled={!isNative || adaptiveBusy != null}
                onClick={toggleAdaptive}
              >
                <span />
              </button>
            </div>
            <button
              className="adaptive-propose-button"
              type="button"
              disabled={!isNative || adaptiveBusy != null}
              onClick={proposeAdaptive}
            >
              {adaptiveBusy === "propose" ? "生成中…" : "生成自适应提案"}
            </button>
          </>
        ) : (
          <div className="empty-card">
            {profiles.length === 0 ? "尚无 Profile" : "选择 Profile 以读取设置"}
          </div>
        )}
      </section>

      <section className="panel-section validation-section">
        <header className="section-heading">
          <div>
            <h3>本地质量检查</h3>
            <p>后台评估搜索表现，仅提供建议，不阻止当前 Profile。</p>
          </div>
          <span>{sampleCount} 项</span>
        </header>
        <div className="validation-actions">
          <button
            type="button"
            disabled={!isNative || !profileId || validationBusy != null}
            onClick={() => refreshValidation("generate")}
          >
            {validationBusy === "generate" ? "准备中…" : "更新检查项"}
          </button>
          <button
            className="primary-button"
            type="button"
            disabled={!isNative || !profileId || validationBusy != null}
            onClick={runValidation}
          >
            {validationBusy === "run" ? "启动中…" : "运行本地检查"}
          </button>
          <button
            type="button"
            disabled={!isNative || !profileId || validationBusy != null}
            onClick={() => refreshValidation()}
          >
            {validationBusy === "refresh" ? "刷新中…" : "刷新状态"}
          </button>
          <button
            type="button"
            disabled={!isNative || !profileId || dataBusy != null}
            onClick={exportValidation}
          >
            {dataBusy === "export" ? "导出中…" : "导出 JSON"}
          </button>
          <button
            className="danger-outline"
            type="button"
            disabled={!isNative || !profileId || dataBusy != null}
            onClick={resetValidation}
          >
            {dataBusy === "reset" ? "重置中…" : "重置本地数据"}
          </button>
        </div>
        <div className="validation-runs">
          {validationRuns.length === 0 ? (
            <div className="empty-card">暂无检查记录</div>
          ) : validationRuns.map((run) => (
            <article key={run.runId}>
              <header>
                <div>
                  <strong>{run.status}</strong>
                  <time>{new Date(run.startedAt * 1000).toLocaleString()}</time>
                </div>
                <span className={`status-badge status-badge--${run.status}`}>
                  {run.completedAt ? "已完成" : "后台运行"}
                </span>
              </header>
              {Object.keys(run.metrics).length > 0 && (
                <details>
                  <summary>查看结果摘要</summary>
                  <pre>{JSON.stringify(run.metrics, null, 2)}</pre>
                </details>
              )}
              {run.error && <p className="profile-error">{run.error}</p>}
            </article>
          ))}
        </div>
      </section>

      <section className="panel-section">
        <header className="section-heading">
          <div><h3>决策历史</h3><p>提议、证据、审核决定与应用时间。</p></div>
          <span>{history.length}</span>
        </header>
        <div className="adaptive-history">
          {history.length === 0 ? (
            <div className="empty-card">暂无自适应历史</div>
          ) : history.map((item) => {
            const proposal = adaptiveProposal(item.proposal);
            const canRevert =
              item.decision === "applied" &&
              settings?.adaptivePolicy.appliedHistoryId === item.historyId;
            return (
              <article key={item.historyId}>
                <header>
                  <strong>{item.decision}</strong>
                  <time>{new Date(item.createdAt * 1000).toLocaleString()}</time>
                </header>
                <p>参数版本：{item.parameterVersion}</p>
                {proposal && (
                  <>
                    <div className="proposal-bounds">
                      <span>H {proposal.policy.h.toLocaleString()}</span>
                      <span>M {proposal.policy.m.toLocaleString()}</span>
                      <span>U {proposal.policy.u.toLocaleString()}</span>
                      <span>L {proposal.policy.l.toLocaleString()}</span>
                    </div>
                    <p>
                      置信度：{proposal.confidence}
                      {proposal.largeChange ? " · 较大变化" : ""}
                    </p>
                    {proposal.warnings.length > 0 && (
                      <p className="proposal-warning">
                        {proposal.warnings.join("；")}
                      </p>
                    )}
                  </>
                )}
                {item.decision === "proposed" && (
                  <div className="history-actions">
                    <button
                      className="primary-button"
                      type="button"
                      disabled={adaptiveBusy != null}
                      onClick={() => decideProposal(item, "apply")}
                    >
                      应用提案
                    </button>
                    <button
                      type="button"
                      disabled={adaptiveBusy != null}
                      onClick={() => decideProposal(item, "reject")}
                    >
                      拒绝提案
                    </button>
                  </div>
                )}
                {canRevert && (
                  <div className="history-actions">
                    <button
                      className="danger-outline"
                      type="button"
                      disabled={adaptiveBusy != null}
                      onClick={() => decideProposal(item, "revert")}
                    >
                      回滚此设置
                    </button>
                  </div>
                )}
                <details>
                  <summary>查看提案与证据</summary>
                  <pre>{JSON.stringify({ proposal: item.proposal, evidence: item.evidence }, null, 2)}</pre>
                </details>
              </article>
            );
          })}
        </div>
      </section>
    </div>
  );
}
