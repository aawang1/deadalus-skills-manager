import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AdaptiveHistoryRecord,
  AdaptivePolicyState,
  ApiKeyMetadata,
  CanonicalSnapshot,
  CreateEmbeddingProfileRequest,
  CredentialPurpose,
  EmbeddingJob,
  EmbeddingProfile,
  EmbeddingProfileDefaults,
  EmbeddingProfileSettings,
  FeedbackEvent,
  FeedbackRequest,
  IndexDiff,
  IndexSyncStatus,
  LocalValidationExport,
  LocalValidationMutation,
  PreflightEstimate,
  PreparedProfileChange,
  ProfileChangeRequest,
  ProgressEvent,
  ProviderModelsResponse,
  SemanticSearchResult,
  SkillGraphSnapshot,
  SkillRelationship,
  ToastMessage,
  ValidationRun,
  ValidationSample,
  VectorType,
} from "./types";

export const isNativeRuntime = () => isTauri();

export const api = {
  getCanonicalSkillsSnapshot: () =>
    invoke<CanonicalSnapshot>("get_canonical_skills_snapshot"),
  refreshCanonicalSkillsSnapshot: () =>
    invoke<CanonicalSnapshot>("refresh_canonical_skills_snapshot"),

  listApiKeys: () => invoke<ApiKeyMetadata[]>("list_api_keys"),
  saveApiKey: (provider: string, apiKey: string) =>
    invoke<ApiKeyMetadata>("save_api_key", { provider, apiKey }),
  deleteApiKey: (id: string) =>
    invoke<ApiKeyMetadata[]>("delete_api_key", { id }),
  activateApiKeyForPurpose: (id: string, purpose: CredentialPurpose) =>
    invoke<ApiKeyMetadata[]>("activate_api_key_for_purpose", { id, purpose }),
  listProviderModels: (id: string) =>
    invoke<ProviderModelsResponse>("list_provider_models", { id }),
  setAgentModel: (id: string, model: string) =>
    invoke<ApiKeyMetadata[]>("set_agent_model", { id, model }),

  getEmbeddingProfileDefaults: (provider: string) =>
    invoke<EmbeddingProfileDefaults>("get_embedding_profile_defaults", {
      provider,
    }),
  listEmbeddingProfiles: () =>
    invoke<EmbeddingProfile[]>("list_embedding_profiles"),
  createEmbeddingProfile: (request: CreateEmbeddingProfileRequest) =>
    invoke<EmbeddingProfile>("create_embedding_profile", { request }),
  getActiveEmbeddingProfile: () =>
    invoke<EmbeddingProfile | null>("get_active_embedding_profile"),
  activateReadyEmbeddingProfile: (profileId: string) =>
    invoke<EmbeddingProfile>("activate_ready_embedding_profile", { profileId }),
  getEmbeddingProfileSettings: (profileId: string) =>
    invoke<EmbeddingProfileSettings>("get_embedding_profile_settings", {
      profileId,
    }),
  listEmbeddingAdaptiveHistory: (profileId: string) =>
    invoke<AdaptiveHistoryRecord[]>("list_embedding_adaptive_history", {
      profileId,
    }),
  setLocalAdaptiveEnabled: (profileId: string, enabled: boolean) =>
    invoke<AdaptivePolicyState>("set_local_adaptive_enabled", {
      profileId,
      enabled,
    }),
  proposeAdaptiveSettings: (profileId: string) =>
    invoke<AdaptiveHistoryRecord>("propose_adaptive_settings", { profileId }),
  applyAdaptiveHistory: (historyId: string) =>
    invoke<AdaptivePolicyState>("apply_adaptive_history", { historyId }),
  rejectAdaptiveHistory: (historyId: string) =>
    invoke<AdaptiveHistoryRecord>("reject_adaptive_history", { historyId }),
  revertAdaptiveHistory: (historyId: string) =>
    invoke<AdaptivePolicyState>("revert_adaptive_history", { historyId }),

  // Contracts below are intentionally isolated: the UI can ship before the
  // vectorization worker commands while retaining compile-time request shapes.
  prepareProfileChange: (request: ProfileChangeRequest) =>
    invoke<PreparedProfileChange>("prepare_profile_change", { request }),
  beginProfileRebuildPreflight: (request: ProfileChangeRequest) =>
    invoke<PreflightEstimate>("begin_profile_rebuild_preflight", { request }),
  beginProfileMigrationPreflight: (request: ProfileChangeRequest) =>
    invoke<PreflightEstimate>("begin_profile_migration_preflight", { request }),
  getIndexSyncStatus: () =>
    invoke<IndexSyncStatus>("get_index_sync_status"),
  setIndexAutoUpdate: (enabled: boolean) =>
    invoke<IndexSyncStatus>("set_index_auto_update", { enabled }),
  scanEmbeddingChanges: () =>
    invoke<IndexDiff>("scan_embedding_changes"),
  getIndexDiff: () => invoke<IndexDiff>("get_index_diff"),
  beginFullRebuildPreflight: (profileId?: string) =>
    invoke<PreflightEstimate>("begin_full_rebuild_preflight", { profileId }),
  listEmbeddingJobs: () => invoke<EmbeddingJob[]>("list_embedding_jobs"),
  startEmbeddingJob: (
    strategy: "incremental" | "full_rebuild" | "profile_migration",
    request?: ProfileChangeRequest,
    profileId?: string,
  ) =>
    invoke<EmbeddingJob>("start_embedding_job", {
      strategy,
      request,
      profileId,
    }),
  cancelEmbeddingJob: (jobId: string) =>
    invoke<boolean>("cancel_embedding_job", { jobId }),

  semanticSearch: (
    query: string,
    agentFilter?: string,
    vectorTypes?: VectorType[],
  ) =>
    invoke<SemanticSearchResult[]>("semantic_search", {
      query,
      agentFilter,
      vectorTypes,
    }),
  listSkillRelations: (skillId: string, profileId?: string) =>
    invoke<SkillRelationship[]>("list_skill_relations", {
      skillId,
      profileId,
    }),
  getSkillGraph: (viewId: string) =>
    invoke<SkillGraphSnapshot>("get_skill_graph", { viewId }),

  generateLocalValidationSamples: (profileId?: string) =>
    invoke<ValidationSample[]>("generate_local_validation_samples", {
      profileId,
    }),
  listLocalValidationSamples: (
    profileId?: string,
    split?: ValidationSample["split"],
  ) =>
    invoke<ValidationSample[]>("list_local_validation_samples", {
      profileId,
      split,
    }),
  runLocalValidation: (profileId: string) =>
    invoke<ValidationRun>("run_local_validation", { profileId }),
  listLocalValidationRuns: (profileId?: string) =>
    invoke<ValidationRun[]>("list_local_validation_runs", { profileId }),
  exportLocalValidationData: (profileId?: string) =>
    invoke<LocalValidationExport>("export_local_validation_data", {
      profileId,
    }),
  resetLocalValidationData: (profileId?: string) =>
    invoke<LocalValidationMutation>("reset_local_validation_data", {
      profileId,
    }),
  deleteLocalValidationSample: (sampleId: string) =>
    invoke<boolean>("delete_local_validation_sample", { sampleId }),
  recordFeedbackEvent: (request: FeedbackRequest) =>
    invoke<FeedbackEvent>("record_feedback_event", { request }),
  revertFeedbackEvent: (feedbackId: string) =>
    invoke<FeedbackEvent>("revert_feedback_event", { feedbackId }),
};

export function isUnavailableCommandError(error: unknown): boolean {
  const message = String(error).toLocaleLowerCase();
  return (
    message.includes("not found") ||
    message.includes("unknown command") ||
    message.includes("command") && message.includes("missing")
  );
}

export async function listenToEmbeddingEvents(handlers: {
  onProgress: (event: ProgressEvent) => void;
  onToast: (toast: ToastMessage) => void;
  onEstimate: (estimate: PreflightEstimate) => void;
}): Promise<UnlistenFn> {
  const unlisteners = await Promise.all([
    listen<ProgressEvent>("embedding-job-progress", ({ payload }) =>
      handlers.onProgress(payload),
    ),
    listen<ToastMessage>("embedding-toast", ({ payload }) =>
      handlers.onToast(payload),
    ),
    listen<PreflightEstimate>("embedding-preflight-estimate", ({ payload }) =>
      handlers.onEstimate(payload),
    ),
  ]);
  return () => unlisteners.forEach((unlisten) => unlisten());
}
