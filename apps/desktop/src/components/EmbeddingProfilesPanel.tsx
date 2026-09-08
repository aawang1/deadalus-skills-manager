import { type FormEvent, useEffect, useState } from "react";
import { api } from "../api";
import type {
  ApiKeyMetadata,
  CreateEmbeddingProfileRequest,
  EmbeddingProfile,
  EmbeddingProfileDefaults,
  ProfileChangeRequest,
} from "../types";

interface EmbeddingProfilesPanelProps {
  isNative: boolean;
  keys: ApiKeyMetadata[];
  profiles: EmbeddingProfile[];
  setKeys: (keys: ApiKeyMetadata[]) => void;
  setProfiles: (profiles: EmbeddingProfile[]) => void;
  notify: (type: "info" | "success" | "error", message: string) => void;
  requestProfileSwitch: (request: ProfileChangeRequest) => void;
}

export function EmbeddingProfilesPanel({
  isNative,
  keys,
  profiles,
  setKeys,
  setProfiles,
  notify,
  requestProfileSwitch,
}: EmbeddingProfilesPanelProps) {
  const activeCredential = keys.find((key) => key.isEmbeddingActive);
  const activeProfile = profiles.find((profile) => profile.isActive);
  const [defaults, setDefaults] = useState<EmbeddingProfileDefaults>();
  const [form, setForm] = useState<CreateEmbeddingProfileRequest>();
  const [busy, setBusy] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<EmbeddingProfile>();
  const [deleteError, setDeleteError] = useState("");

  useEffect(() => {
    if (!isNative || !activeCredential) {
      setDefaults(undefined);
      setForm(undefined);
      return;
    }
    let current = true;
    api.getEmbeddingProfileDefaults(activeCredential.provider)
      .then((value) => {
        if (!current) return;
        setDefaults(value);
        setForm({
          ...value,
          credentialId: activeCredential.id,
        });
      })
      .catch((error) => notify("error", String(error)));
    return () => {
      current = false;
    };
  }, [activeCredential?.id, activeCredential?.provider, isNative]);

  useEffect(() => {
    if (!pendingDelete) return;
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
    if (!form || !isNative) return;
    setBusy(true);
    try {
      const profile = await api.createEmbeddingProfile(form);
      setProfiles([profile, ...profiles]);
      notify("success", "Profile 草稿已创建，可由索引任务构建为 Ready。");
    } catch (error) {
      notify("error", String(error));
    } finally {
      setBusy(false);
    }
  };

  const activate = async (profile: EmbeddingProfile) => {
    if (!isNative || profile.status !== "ready") return;
    const targetCredential = keys.find(
      (key) => key.id === profile.credentialId,
    );
    const commit = async () => {
      setBusy(true);
      try {
        if (targetCredential && !targetCredential.isEmbeddingActive) {
          setKeys(
            await api.activateApiKeyForPurpose(targetCredential.id, "embedding"),
          );
        }
        const activated = await api.activateReadyEmbeddingProfile(
          profile.profileId,
        );
        setProfiles(
          profiles.map((item) => ({
            ...item,
            isActive: item.profileId === activated.profileId,
            status:
              item.profileId === activated.profileId
                ? activated.status
                : item.status === "active"
                  ? "ready"
                  : item.status,
          })),
        );
        notify("success", "Embedding Profile 已激活。");
      } finally {
        setBusy(false);
      }
    };

    if (
      activeProfile &&
      (activeProfile.provider !== profile.provider ||
        activeProfile.model !== profile.model ||
        activeProfile.dimensions !== profile.dimensions)
    ) {
      requestProfileSwitch(
        {
          sourceProfileId: activeProfile.profileId,
          targetProfileId: profile.profileId,
          targetCredentialId: profile.credentialId,
          provider: profile.provider,
          model: profile.model,
          reason: "profile",
        },
      );
      return;
    }
    try {
      await commit();
    } catch (error) {
      notify("error", String(error));
    }
  };

  const requestDeleteProfile = (profile: EmbeddingProfile) => {
    if (!isNative || busy || profile.isActive) return;
    setDeleteError("");
    setPendingDelete(profile);
  };

  const closeDeleteProfile = () => {
    if (busy) return;
    setPendingDelete(undefined);
    setDeleteError("");
  };

  const confirmDeleteProfile = async () => {
    if (!isNative || !pendingDelete || busy) return;
    const profile = pendingDelete;
    setBusy(true);
    setDeleteError("");
    try {
      const deleted = await api.deleteEmbeddingProfile(profile.profileId);
      if (!deleted) {
        setDeleteError("该 Profile 已不存在。请关闭弹窗并刷新列表。");
        return;
      }
      setProfiles(profiles.filter((item) => item.profileId !== profile.profileId));
      setPendingDelete(undefined);
      notify("success", "Embedding Profile 已删除。");
    } catch (error) {
      const message = String(error);
      setDeleteError(message);
      notify("error", message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-panel profiles-panel">
      <section className="panel-section profile-create">
        <header className="section-heading">
          <div><h3>创建 Embedding Profile</h3><p>配置与索引语义绑定，创建后不会静默覆盖。</p></div>
          {defaults && <span className="status-badge">已载入默认值</span>}
        </header>
        {!isNative ? (
          <div className="inline-notice">浏览器预览为只读，创建操作已禁用。</div>
        ) : !activeCredential ? (
          <div className="inline-notice">
            请先在“凭据管理”中启用 OpenAI 或 Qwen Embedding 凭据。
          </div>
        ) : form ? (
          <form className="profile-form" onSubmit={createProfile}>
            <label>
              <span>Provider</span>
              <input value={form.provider} disabled />
            </label>
            <label>
              <span>Model</span>
              <input
                value={form.model}
                onChange={(event) =>
                  setForm({ ...form, model: event.currentTarget.value })
                }
              />
            </label>
            <label>
              <span>Version</span>
              <input
                value={form.version}
                onChange={(event) =>
                  setForm({ ...form, version: event.currentTarget.value })
                }
              />
            </label>
            <label>
              <span>Dimensions</span>
              <input
                type="number"
                min={1}
                max={65536}
                value={form.dimensions}
                onChange={(event) =>
                  setForm({
                    ...form,
                    dimensions: event.currentTarget.valueAsNumber,
                  })
                }
              />
            </label>
            <button
              className="primary-button"
              type="submit"
              disabled={
                busy ||
                !form.model.trim() ||
                !form.version.trim() ||
                !Number.isFinite(form.dimensions)
              }
            >
              {busy ? "创建中…" : "创建 Profile"}
            </button>
          </form>
        ) : (
          <div className="inline-notice">正在读取 Provider 默认配置…</div>
        )}
      </section>

      <section className="panel-section">
        <header className="section-heading">
          <div><h3>Profiles</h3><p>仅 Ready 状态可以激活。</p></div>
          <span>{profiles.length}</span>
        </header>
        <div className="profile-list">
          {profiles.length === 0 ? (
            <div className="empty-card">尚未创建 Embedding Profile</div>
          ) : profiles.map((profile) => (
            <article className="profile-card" key={profile.profileId}>
              <header>
                <div>
                  <h4>{profile.model}</h4>
                  <code>{profile.profileId}</code>
                </div>
                <div className="profile-badges">
                  {profile.isActive && (
                    <span className="status-badge status-badge--active">ACTIVE</span>
                  )}
                  <span className={`status-badge status-badge--${profile.status}`}>
                    {profile.status.toUpperCase()}
                  </span>
                  <button
                    className="profile-delete-button"
                    type="button"
                    aria-label={`删除 ${profile.model} Profile`}
                    title={profile.isActive ? "活动 Profile 不能删除" : "删除 Profile"}
                    disabled={!isNative || busy || profile.isActive}
                    onClick={() => requestDeleteProfile(profile)}
                  >
                    ×
                  </button>
                </div>
              </header>
              <dl>
                <div><dt>Provider</dt><dd>{profile.provider}</dd></div>
                <div><dt>Version</dt><dd>{profile.modelVersion}</dd></div>
                <div><dt>Dimensions</dt><dd>{profile.dimensions}</dd></div>
                <div><dt>Schema</dt><dd>{profile.inputSchemaVersion}</dd></div>
              </dl>
              {profile.error && <p className="profile-error">{profile.error}</p>}
              <button
                type="button"
                disabled={
                  !isNative ||
                  busy ||
                  profile.isActive ||
                  profile.status !== "ready"
                }
                title={
                  profile.status !== "ready"
                    ? "索引构建完成后才能激活"
                    : undefined
                }
                onClick={() => activate(profile)}
              >
                {profile.isActive ? "当前活动" : "激活 Ready Profile"}
              </button>
            </article>
          ))}
        </div>
      </section>

      {pendingDelete && (
        <div
          className="nested-modal-backdrop"
          role="presentation"
          onMouseDown={closeDeleteProfile}
        >
          <section
            className="profile-delete-modal"
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="profile-delete-title"
            aria-describedby="profile-delete-description"
            onMouseDown={(event) => event.stopPropagation()}
          >
            <header>
              <div>
                <p className="eyebrow">DESTRUCTIVE ACTION</p>
                <h3 id="profile-delete-title">确认删除 Embedding Profile</h3>
              </div>
              <button
                className="close-button"
                type="button"
                aria-label="关闭删除确认"
                disabled={busy}
                onClick={closeDeleteProfile}
              >
                ×
              </button>
            </header>
            <strong>{pendingDelete.model}</strong>
            <code>{pendingDelete.profileId}</code>
            <p id="profile-delete-description">
              删除后，该 Profile 的索引、向量、Jobs、自适应参数与本地验证数据将一并清除。此操作无法撤销。
            </p>
            {deleteError && (
              <p className="profile-delete-modal__error" role="alert">
                {deleteError}
              </p>
            )}
            <footer>
              <button type="button" disabled={busy} onClick={closeDeleteProfile} autoFocus>
                取消
              </button>
              <button
                className="danger-button"
                type="button"
                disabled={busy}
                onClick={confirmDeleteProfile}
              >
                {busy ? "删除中…" : "确认删除"}
              </button>
            </footer>
          </section>
        </div>
      )}
    </div>
  );
}
