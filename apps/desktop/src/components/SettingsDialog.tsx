import {
  type RefObject,
  useCallback,
  useEffect,
  useState,
} from "react";
import { api, isUnavailableCommandError, listenToEmbeddingEvents } from "../api";
import type {
  ApiKeyMetadata,
  EmbeddingProfile,
  PreflightEstimate,
  ProfileChangeRequest,
  ProgressEvent,
  ToastMessage,
} from "../types";
import { AdaptivePanel } from "./AdaptivePanel";
import { CredentialsPanel } from "./CredentialsPanel";
import { EmbeddingProfilesPanel } from "./EmbeddingProfilesPanel";
import { IndexSyncPanel } from "./IndexSyncPanel";
import { PreflightEstimateModal } from "./PreflightEstimateModal";
import { ProfileSwitchModal } from "./ProfileSwitchModal";

type SettingsTab = "credentials" | "profiles" | "index" | "adaptive";

const tabs: Array<{ id: SettingsTab; label: string }> = [
  { id: "credentials", label: "凭据管理" },
  { id: "profiles", label: "Embedding Profiles" },
  { id: "index", label: "索引与同步" },
  { id: "adaptive", label: "自适应" },
];

interface PendingSwitch {
  request: ProfileChangeRequest;
}

interface EstimateExecution {
  strategy: "incremental" | "full_rebuild" | "profile_migration";
  request?: ProfileChangeRequest;
  profileId?: string;
}

interface SettingsDialogProps {
  isNative: boolean;
  keyInputRef: RefObject<HTMLInputElement | null>;
  onClose: () => void;
}

export function SettingsDialog({
  isNative,
  keyInputRef,
  onClose,
}: SettingsDialogProps) {
  const [tab, setTab] = useState<SettingsTab>("credentials");
  const [keys, setKeys] = useState<ApiKeyMetadata[]>([]);
  const [profiles, setProfiles] = useState<EmbeddingProfile[]>([]);
  const [loading, setLoading] = useState(isNative);
  const [toasts, setToasts] = useState<ToastMessage[]>([]);
  const [progress, setProgress] = useState<ProgressEvent>();
  const [jobRefreshVersion, setJobRefreshVersion] = useState(0);
  const [pendingSwitch, setPendingSwitch] = useState<PendingSwitch>();
  const [switchBusy, setSwitchBusy] = useState(false);
  const [estimate, setEstimate] = useState<PreflightEstimate>();
  const [estimateExecution, setEstimateExecution] =
    useState<EstimateExecution>();

  const notify = useCallback(
    (type: ToastMessage["type"], message: string) => {
      const toast = {
        id: `${Date.now()}-${Math.random().toString(16).slice(2)}`,
        type,
        message,
      };
      setToasts((current) => [...current.slice(-2), toast]);
      window.setTimeout(() => {
        setToasts((current) => current.filter((item) => item.id !== toast.id));
      }, 5200);
    },
    [],
  );

  useEffect(() => {
    if (!isNative) {
      setLoading(false);
      return;
    }
    let current = true;
    Promise.all([api.listApiKeys(), api.listEmbeddingProfiles()])
      .then(([nextKeys, nextProfiles]) => {
        if (!current) return;
        setKeys(nextKeys);
        setProfiles(nextProfiles);
      })
      .catch((error) => current && notify("error", String(error)))
      .finally(() => current && setLoading(false));
    return () => {
      current = false;
    };
  }, [isNative, notify]);

  useEffect(() => {
    if (!isNative) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    listenToEmbeddingEvents({
      onProgress: (event) => {
        setProgress(event);
        setJobRefreshVersion((version) => version + 1);
        if (event.total > 0 && event.completed >= event.total) {
          api.listEmbeddingProfiles().then(setProfiles).catch(() => undefined);
        }
      },
      onToast: (toast) => {
        notify(toast.type, toast.message);
        setJobRefreshVersion((version) => version + 1);
      },
      onEstimate: setEstimate,
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    });
    return () => {
      disposed = true;
      stop?.();
    };
  }, [isNative, notify]);

  useEffect(() => {
    const onEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (estimate) {
        setEstimate(undefined);
        setEstimateExecution(undefined);
      } else if (pendingSwitch) {
        setPendingSwitch(undefined);
      } else {
        onClose();
      }
    };
    window.addEventListener("keydown", onEscape);
    return () => window.removeEventListener("keydown", onEscape);
  }, [estimate, onClose, pendingSwitch]);

  const requestProfileSwitch = useCallback(
    (request: ProfileChangeRequest) => {
      setPendingSwitch({ request });
    },
    [],
  );

  const prepareSwitch = async (strategy: "rebuild" | "migration") => {
    if (!pendingSwitch) return;
    setSwitchBusy(true);
    try {
      try {
        const prepared = await api.prepareProfileChange(pendingSwitch.request);
        if (!prepared.compatible && prepared.reason) {
          notify("info", prepared.reason);
        }
      } catch (error) {
        if (!isUnavailableCommandError(error)) throw error;
      }
      const nextEstimate =
        strategy === "rebuild"
          ? await api.beginProfileRebuildPreflight(pendingSwitch.request)
          : await api.beginProfileMigrationPreflight(pendingSwitch.request);
      setEstimate(nextEstimate);
      setEstimateExecution({
        strategy:
          strategy === "rebuild" ? "full_rebuild" : "profile_migration",
        request: pendingSwitch.request,
        profileId: pendingSwitch.request.targetProfileId,
      });
      setPendingSwitch(undefined);
    } catch (error) {
      if (isUnavailableCommandError(error)) {
        notify(
          "info",
          "Profile 迁移预检尚未在当前后端启用，当前选择保持不变。",
        );
      } else {
        notify("error", String(error));
      }
    } finally {
      setSwitchBusy(false);
    }
  };

  const confirmEstimate = async () => {
    const execution = estimateExecution;
    setEstimate(undefined);
    setEstimateExecution(undefined);
    try {
      if (execution) {
        const job = await api.startEmbeddingJob(
          execution.strategy,
          execution.request,
          execution.profileId,
        );
        notify("success", `任务已启动：${job.jobId}`);
      }
    } catch (error) {
      notify("error", String(error));
    }
  };

  const openEstimate = (
    nextEstimate: PreflightEstimate,
    execution?: EstimateExecution,
  ) => {
    setEstimate(nextEstimate);
    setEstimateExecution(execution);
  };

  const activeProfile = profiles.find((profile) => profile.isActive);

  return (
    <div className="dialog-backdrop" role="presentation">
      <section
        className="settings-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-dialog-title"
      >
        <header className="settings-dialog__header">
          <div>
            <p className="eyebrow">LOCAL INTELLIGENCE</p>
            <h2 id="settings-dialog-title">Credentials &amp; Profiles</h2>
          </div>
          <div className="dialog-runtime">
            <span className={isNative ? "is-ready" : ""}>
              {isNative ? "NATIVE" : "PREVIEW"}
            </span>
            <button
              className="close-button"
              type="button"
              aria-label="关闭设置"
              onClick={onClose}
            >
              ×
            </button>
          </div>
        </header>

        <nav className="settings-tabs" aria-label="设置分区">
          {tabs.map((item) => (
            <button
              key={item.id}
              type="button"
              className={tab === item.id ? "is-active" : ""}
              aria-current={tab === item.id ? "page" : undefined}
              onClick={() => setTab(item.id)}
            >
              {item.label}
            </button>
          ))}
        </nav>

        {progress && (
          <div className="global-progress" role="status">
            <div>
              <span>{progress.label}</span>
              <strong>{progress.completed} / {progress.total}</strong>
            </div>
            <progress value={progress.completed} max={progress.total || 1} />
          </div>
        )}

        <div className="settings-dialog__body">
          {loading ? (
            <div className="settings-loading">
              <span className="spinner" />
              正在读取本地配置…
            </div>
          ) : (
            <>
              {tab === "credentials" && (
                <CredentialsPanel
                  isNative={isNative}
                  keys={keys}
                  setKeys={setKeys}
                  keyInputRef={keyInputRef}
                  activeProfileProvider={activeProfile?.provider}
                  notify={notify}
                  requestProfileSwitch={requestProfileSwitch}
                />
              )}
              {tab === "profiles" && (
                <EmbeddingProfilesPanel
                  isNative={isNative}
                  keys={keys}
                  profiles={profiles}
                  setKeys={setKeys}
                  setProfiles={setProfiles}
                  notify={notify}
                  requestProfileSwitch={requestProfileSwitch}
                />
              )}
              {tab === "index" && (
                <IndexSyncPanel
                  isNative={isNative}
                  activeProfileId={activeProfile?.profileId}
                  profiles={profiles}
                  onEstimate={openEstimate}
                  refreshVersion={jobRefreshVersion}
                  notify={notify}
                />
              )}
              {tab === "adaptive" && (
                <AdaptivePanel
                  isNative={isNative}
                  profiles={profiles}
                  notify={notify}
                />
              )}
            </>
          )}
        </div>
      </section>

      <div className="toast-stack" aria-live="polite">
        {toasts.map((toast) => (
          <div key={toast.id} className={`toast toast--${toast.type}`}>
            {toast.message}
          </div>
        ))}
      </div>

      {pendingSwitch && (
        <ProfileSwitchModal
          request={pendingSwitch.request}
          busy={switchBusy}
          onGlobalUpdate={() => prepareSwitch("rebuild")}
          onRecommendedMigration={() => prepareSwitch("migration")}
          onCancel={() => setPendingSwitch(undefined)}
        />
      )}
      {estimate && (
        <PreflightEstimateModal
          estimate={estimate}
          onConfirm={estimateExecution ? confirmEstimate : undefined}
          onClose={() => {
            setEstimate(undefined);
            setEstimateExecution(undefined);
          }}
        />
      )}
    </div>
  );
}
