import { tr, displayStatus } from "../i18n";
import { type FormEvent, useEffect, useState } from "react";
import { api } from "../api";
import type { ApiKeyMetadata, EmbeddingProfile, EmbeddingProfileDefaults, ProfileChangeRequest, } from "../types";
interface EmbeddingProfilesPanelProps {
    isNative: boolean;
    keys: ApiKeyMetadata[];
    profiles: EmbeddingProfile[];
    setKeys: (keys: ApiKeyMetadata[]) => void;
    setProfiles: (profiles: EmbeddingProfile[]) => void;
    notify: (type: "info" | "success" | "error", message: string) => void;
    requestProfileSwitch: (request: ProfileChangeRequest) => void;
}
export function EmbeddingProfilesPanel({ isNative, keys, profiles, setKeys, setProfiles, notify, requestProfileSwitch, }: EmbeddingProfilesPanelProps) {
    const activeCredential = keys.find((key) => key.isEmbeddingActive && key.purpose !== "agent");
    const agentCredential = keys.find((key) => key.isAgentActive && key.purpose !== "embedding");
    const activeProfile = profiles.find((profile) => profile.isActive);
    const [defaults, setDefaults] = useState<EmbeddingProfileDefaults>();
    const [name, setName] = useState("");
    const [description, setDescription] = useState("");
    const [defaultsError, setDefaultsError] = useState("");
    const [busy, setBusy] = useState(false);
    const [pendingDelete, setPendingDelete] = useState<EmbeddingProfile>();
    const [deleteError, setDeleteError] = useState("");
    useEffect(() => {
        setDefaults(undefined);
        setDefaultsError("");
        if (!isNative || !activeCredential) {
            return;
        }
        let current = true;
        api.getEmbeddingProfileDefaults(activeCredential.provider)
            .then((value) => {
            if (!current)
                return;
            setDefaults(value);
        })
            .catch((error) => { if (current) setDefaultsError(String(error)); });
        return () => {
            current = false;
        };
    }, [activeCredential?.id, activeCredential?.provider, isNative]);
    useEffect(() => {
        if (!pendingDelete)
            return;
        const handleKeyDown = (event: KeyboardEvent) => {
            if (event.key === "Escape" && !busy) {
                setPendingDelete(undefined);
                setDeleteError("");
            }
        };
        document.addEventListener("keydown", handleKeyDown);
        return () => document.removeEventListener("keydown", handleKeyDown);
    }, [busy, pendingDelete]);
    const createProfile = async (event: FormEvent) => {
        event.preventDefault();
        if (!activeCredential || !defaults || !name.trim() || !isNative || busy)
            return;
        setBusy(true);
        try {
            const profile = await api.createEmbeddingProfile({ name: name.trim(), description: description.trim(), credentialId: activeCredential.id });
            setProfiles([profile, ...profiles]);
            setName("");
            setDescription("");
            notify("success", tr("Profile 草稿已创建，可由索引任务构建为 Ready。"));
        }
        catch (error) {
            notify("error", String(error));
        }
        finally {
            setBusy(false);
        }
    };
    const activate = async (profile: EmbeddingProfile) => {
        if (!isNative || profile.status !== "ready")
            return;
        const targetCredential = keys.find((key) => key.id === profile.credentialId);
        const commit = async () => {
            setBusy(true);
            try {
                if (targetCredential && !targetCredential.isEmbeddingActive) {
                    setKeys(await api.activateApiKeyForPurpose(targetCredential.id, "embedding"));
                }
                const activated = await api.activateReadyEmbeddingProfile(profile.profileId);
                setProfiles(profiles.map((item) => ({
                    ...item,
                    isActive: item.profileId === activated.profileId,
                    status: item.profileId === activated.profileId
                        ? activated.status
                        : item.status === "active"
                            ? "ready"
                            : item.status,
                })));
                notify("success", tr("Embedding Profile 已激活。"));
            }
            finally {
                setBusy(false);
            }
        };
        if (activeProfile &&
            (activeProfile.provider !== profile.provider ||
                activeProfile.model !== profile.model ||
                activeProfile.dimensions !== profile.dimensions)) {
            requestProfileSwitch({
                sourceProfileId: activeProfile.profileId,
                targetProfileId: profile.profileId,
                targetCredentialId: profile.credentialId,
                provider: profile.provider,
                model: profile.model,
                reason: "profile",
            });
            return;
        }
        try {
            await commit();
        }
        catch (error) {
            notify("error", String(error));
        }
    };
    const requestDeleteProfile = (profile: EmbeddingProfile) => {
        if (!isNative || busy || profile.isActive)
            return;
        setDeleteError("");
        setPendingDelete(profile);
    };
    const closeDeleteProfile = () => {
        if (busy)
            return;
        setPendingDelete(undefined);
        setDeleteError("");
    };
    const confirmDeleteProfile = async () => {
        if (!isNative || !pendingDelete || busy)
            return;
        const profile = pendingDelete;
        setBusy(true);
        setDeleteError("");
        try {
            const deleted = await api.deleteEmbeddingProfile(profile.profileId);
            if (!deleted) {
                setDeleteError(tr("该 Profile 已不存在。请关闭弹窗并刷新列表。"));
                return;
            }
            setProfiles(profiles.filter((item) => item.profileId !== profile.profileId));
            setPendingDelete(undefined);
            notify("success", tr("Embedding Profile 已删除。"));
        }
        catch (error) {
            const message = String(error);
            setDeleteError(message);
            notify("error", message);
        }
        finally {
            setBusy(false);
        }
    };
    return (<div className="settings-panel profiles-panel">
      <section className="panel-section profile-create">
        <header className="section-heading">
          <div><h3>{tr("创建 Embedding Profile")}</h3><p>{tr("配置与索引语义绑定，创建后不会静默覆盖。")}</p></div>
          {defaults && <span className="status-badge">{tr("已载入默认值")}</span>}
        </header>
        {!isNative ? (<div className="inline-notice">{tr("浏览器预览为只读，创建操作已禁用。")}</div>) : !activeCredential ? (<div className="inline-notice">{tr("请先在“apikeys管理”中启用 OpenAI 或 Qwen Embedding 凭据。")}</div>) : defaults ? (<form className="profile-form" onSubmit={createProfile}>
            <label>
              <span>{tr("名称")}</span>
              <input value={name} maxLength={100} required disabled={busy} onChange={(event) => setName(event.currentTarget.value)}/>
            </label>
            <label>
              <span>{tr("简介")}</span>
              <input value={description} maxLength={1000} disabled={busy} onChange={(event) => setDescription(event.currentTarget.value)}/>
            </label>
            <button className="primary-button" type="submit" disabled={busy || !name.trim()}>
              {busy ? tr("创建中…") : tr("创建 Profile")}
            </button>
          </form>) : (<div className="inline-notice" role={defaultsError ? "alert" : "status"}>{defaultsError || tr("正在读取 Provider 默认配置…")}</div>)}
        {defaults && activeCredential && <p className="credential-hint">{tr("自动配置：{0} / {1} / {2} 维", defaults.provider, defaults.model, defaults.dimensions)} · {activeCredential.maskedKey}</p>}
        {agentCredential ? <p className="credential-hint">{tr("LLM 分析使用当前 Agent 凭据：{0}；模型：{1}", agentCredential.provider, agentCredential.agentModel || tr("按服务商自动选择"))}</p> : <p className="credential-hint">{tr("开始向量化前，请先启用 Agent 凭据用于 LLM 分析。")}</p>}
      </section>

      <section className="panel-section">
        <header className="section-heading">
          <div><h3>{tr("配置列表")}</h3><p>{tr("仅 Ready 状态可以激活。")}</p></div>
          <span>{profiles.length}</span>
        </header>
        <div className="profile-list">
          {profiles.length === 0 ? (<div className="empty-card">{tr("尚未创建 Embedding Profile")}</div>) : profiles.map((profile) => (<article className="profile-card" key={profile.profileId}>
              <header>
                <div>
                  <h4>{profile.name || profile.model}</h4>
                </div>
                <div className="profile-badges">
                  {profile.isActive && (<span className="status-badge status-badge--active">{tr("已激活")}</span>)}
                  <span className={`status-badge status-badge--${displayStatus(profile.status)}`}>
                    {displayStatus(profile.status)}
                  </span>
                  <button className="profile-delete-button" type="button" aria-label={tr("删除 {0} Profile", profile.name || profile.model)} title={profile.isActive ? tr("活动 Profile 不能删除") : tr("删除 Profile")} disabled={!isNative || busy || profile.isActive} onClick={() => requestDeleteProfile(profile)}>
                    ×
                  </button>
                </div>
              </header>
              {profile.description && <p className="credential-hint profile-description">{profile.description}</p>}
              <dl>
                <div><dt>{tr("服务商")}</dt><dd>{profile.provider}</dd></div>
                <div><dt>{tr("模型")}</dt><dd>{profile.model}</dd></div>
              </dl>
              {profile.error && <p className="profile-error">{profile.error}</p>}
              <button type="button" disabled={!isNative ||
                busy ||
                profile.isActive ||
                profile.status !== "ready"} title={profile.status !== "ready"
                ? tr("索引构建完成后才能激活") : undefined} onClick={() => activate(profile)}>
                {profile.isActive ? tr("当前活动") : tr("激活 Ready Profile")}
              </button>
            </article>))}
        </div>
      </section>

      {pendingDelete && (<div className="nested-modal-backdrop" role="presentation" onMouseDown={closeDeleteProfile}>
          <section className="profile-delete-modal" role="alertdialog" aria-modal="true" aria-labelledby="profile-delete-title" aria-describedby="profile-delete-description" onMouseDown={(event) => event.stopPropagation()}>
            <header>
              <div>
                <p className="eyebrow">{tr("高风险操作")}</p>
                <h3 id="profile-delete-title">{tr("确认删除 Embedding Profile")}</h3>
              </div>
              <button className="close-button" type="button" aria-label={tr("关闭删除确认")} disabled={busy} onClick={closeDeleteProfile}>
                ×
              </button>
            </header>
            <strong>{pendingDelete.name || pendingDelete.model}</strong>
            <code>{pendingDelete.profileId}</code>
            <p id="profile-delete-description">{tr("删除后，该 Profile 的索引、向量、Jobs、自适应参数与本地验证数据将一并清除。此操作无法撤销。")}</p>
            {deleteError && (<p className="profile-delete-modal__error" role="alert">
                {deleteError}
              </p>)}
            <footer>
              <button type="button" disabled={busy} onClick={closeDeleteProfile} autoFocus>{tr("取消")}</button>
              <button className="danger-button" type="button" disabled={busy} onClick={confirmDeleteProfile}>
                {busy ? tr("删除中…") : tr("确认删除")}
              </button>
            </footer>
          </section>
        </div>)}
    </div>);
}
