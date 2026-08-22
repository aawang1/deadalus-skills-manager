import type { ProfileChangeRequest } from "../types";

interface ProfileSwitchModalProps {
  request: ProfileChangeRequest;
  busy?: boolean;
  onGlobalUpdate: () => void;
  onRecommendedMigration: () => void;
  onCancel: () => void;
}

export function ProfileSwitchModal({
  request,
  busy = false,
  onGlobalUpdate,
  onRecommendedMigration,
  onCancel,
}: ProfileSwitchModalProps) {
  return (
    <div className="nested-modal-backdrop" role="presentation">
      <section
        className="profile-switch-modal"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="profile-switch-title"
      >
        <button
          className="close-button modal-close"
          type="button"
          aria-label="取消模型改变"
          disabled={busy}
          onClick={onCancel}
        >
          ×
        </button>
        <p className="eyebrow">PROFILE COMPATIBILITY</p>
        <h3 id="profile-switch-title">需要更新 Embedding Profile</h3>
        <p>
          {request.provider.toUpperCase()} 凭据或 Profile 与当前索引不兼容。
          请选择全量重建，或保留现有关系并执行推荐移植。
        </p>
        <div className="profile-switch-actions">
          <button
            className="primary-button"
            type="button"
            disabled={busy}
            onClick={onGlobalUpdate}
          >
            全局更新
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={onRecommendedMigration}
          >
            推荐移植
          </button>
          <button type="button" disabled={busy} onClick={onCancel}>
            取消模型改变
          </button>
        </div>
      </section>
    </div>
  );
}
