export type AgentId = "claude-code" | "cursor" | "codex";
export type ViewId = "all" | AgentId;
export type ProviderId = "anthropic" | "openai" | "deepseek" | "qwen";
export type CredentialPurpose = "agent" | "embedding";

export interface ApiKeyMetadata {
  id: string;
  provider: ProviderId;
  maskedKey: string;
  savedAt: number;
  isAgentActive: boolean;
  isEmbeddingActive: boolean;
  agentModel?: string;
}

export interface ProviderModel {
  id: string;
  displayName: string;
  ownedBy?: string;
}

export interface ProviderModelsResponse {
  credentialId: string;
  provider: ProviderId;
  models: ProviderModel[];
}

export interface InstalledSkill {
  skillId: string;
  name: string;
  description?: string;
  path: string;
  sourcePath: string;
  scope: "user" | "system";
  isBuiltIn: boolean;
  enabledAgents: AgentId[];
  inLibrary: boolean;
  libraryPath?: string;
}

export interface AgentSkillsResponse {
  agent: ViewId;
  officialDocumentation: string;
  searchedPaths: string[];
  skills: InstalledSkill[];
  warnings: string[];
}

export interface CanonicalSnapshot {
  snapshotId: string;
  searchedPaths: string[];
  warnings: string[];
  skills: InstalledSkill[];
}

export type ProfileStatus =
  | "draft"
  | "building"
  | "ready"
  | "active"
  | "failed"
  | "archived";

export interface EmbeddingProfile {
  profileId: string;
  provider: ProviderId;
  model: string;
  modelVersion: string;
  dimensions: number;
  inputSchemaVersion: string;
  chunkPolicyVersion: string;
  tokenizer?: string;
  credentialId?: string;
  status: ProfileStatus;
  isActive: boolean;
  createdAt: number;
  activatedAt?: number;
  error?: string;
}

export interface EmbeddingProfileDefaults {
  provider: ProviderId;
  model: string;
  version: string;
  dimensions: number;
}

export interface CreateEmbeddingProfileRequest {
  provider: ProviderId;
  model: string;
  version: string;
  dimensions: number;
  credentialId: string;
}

export interface AdaptiveHistoryRecord {
  historyId: string;
  profileId: string;
  parameterVersion: string;
  proposal: unknown;
  evidence: unknown;
  decision: string;
  createdAt: number;
  appliedAt?: number;
}

export interface AdaptivePolicyState {
  profileId: string;
  h: number;
  m: number;
  u: number;
  l: number;
  localUpdateEnabled: boolean;
  policyVersion: string;
  limitSource: string;
  limitSourceVersion: string;
  appliedHistoryId?: string;
  effectiveAfter: number;
  updatedAt: number;
}

export interface AdaptiveProposal {
  policy: AdaptivePolicyState;
  requiresReview: boolean;
  largeChange: boolean;
  effectiveFor: string;
  costDiagnostic: unknown;
  costVeto: boolean;
  confidence: string;
  warnings: string[];
}

export interface EmbeddingProfileSettings {
  profile: EmbeddingProfile;
  defaults: EmbeddingProfileDefaults;
  credentialReady: boolean;
  adaptivePolicy: AdaptivePolicyState;
  adaptiveHistory: AdaptiveHistoryRecord[];
}

export interface PreflightEstimate {
  skillCount: number;
  fileCount: number;
  embeddableTextCount: number;
  parentCount: number;
  chunkCountLow: number;
  chunkCountHigh: number;
  analysisTokensLow: number;
  analysisTokensHigh: number;
  embeddingTokensLow: number;
  embeddingTokensHigh: number;
  estimatedCostLow?: number;
  estimatedCostHigh?: number;
  provider: string;
  model: string;
  pricingBasis?: string;
  estimatedAt: number;
  confidence: "low" | "medium" | "high";
  missingReasons: string[];
}

export interface EmbeddingJob {
  jobId: string;
  profileId: string;
  kind: "incremental" | "full_rebuild" | "profile_migration" | "validation";
  status: "pending" | "running" | "paused" | "completed" | "failed" | "cancelled";
  totalItems: number;
  completedItems: number;
  estimatedTokens: number;
  actualTokens: number;
  estimatedCostLow?: number;
  estimatedCostHigh?: number;
  createdAt: number;
  updatedAt: number;
  error?: string;
}

export interface IndexSyncStatus {
  autoUpdate: boolean;
  ignoreBuiltInSkills: boolean;
  indexedSkills: number;
  pendingChanges: number;
  lastSyncedAt?: number;
  activeProfileId?: string;
}

export interface IndexDiff {
  added: number;
  changed: number;
  removed: number;
  unchanged: number;
}

export interface ProfileChangeRequest {
  sourceProfileId?: string;
  targetProfileId?: string;
  targetCredentialId?: string;
  provider: ProviderId;
  model?: string;
  reason: "credential" | "profile";
}

export interface PreparedProfileChange {
  compatible: boolean;
  reason?: string;
  request: ProfileChangeRequest;
  estimate?: PreflightEstimate;
}

export interface ProgressEvent {
  jobId?: string;
  label: string;
  completed: number;
  total: number;
}

export interface ToastMessage {
  id: string;
  type: "info" | "success" | "error";
  message: string;
}

export type VectorType =
  | "overall_function"
  | "trigger"
  | "workflow"
  | "resource"
  | "general";

export interface SearchEvidence {
  embeddingId: string;
  vectorType: VectorType;
  level: "parent" | "chunk";
  chunkId?: string;
  rawScore: number;
  expired: boolean;
  headingPath?: string;
  sourceFile?: string;
}

export interface SemanticSearchResult {
  skillId: string;
  score: number;
  parentScore: number;
  chunkScore?: number;
  expired: boolean;
  matchedTypes: VectorType[];
  evidence: SearchEvidence[];
}

export type RelationshipType =
  | "similar_to"
  | "overlaps_with"
  | "conflicts_with"
  | "duplicate_candidate"
  | "supersedes"
  | "depends_on"
  | "reads_reference"
  | "runs_script"
  | "uses_asset"
  | "located_in";

export interface SkillRelationship {
  relationId: string;
  sourceSkillId: string;
  targetSkillId: string;
  relationshipType: RelationshipType;
  vectorType?: VectorType;
  sourceProfileId?: string;
  targetProfileId?: string;
  score?: number;
  state:
    | "carried_fact"
    | "human_confirmed"
    | "human_rejected"
    | "human_reverted"
    | "migration_hint"
    | "revalidated"
    | "rejected"
    | "conflict";
  evidence: unknown;
  createdAt: number;
  validatedAt?: number;
}

export interface SkillGraphSnapshot {
  graphVersion: string;
  layoutVersion: string;
  profileId?: string;
  viewId: ViewId;
  nodes: SkillGraphNode[];
  clusters: SkillGraphCluster[];
  edges: SkillGraphEdge[];
  proximities: SkillGraphProximity[];
  excludedUnreadyCount: number;
  excludedUnconnectedCount: number;
}

export interface SkillGraphNode {
  skillId: string;
  name: string;
  description?: string;
  path: string;
  enabledAgents: AgentId[];
  clusterId?: string;
  centrality: number;
  superseded: boolean;
}

export interface SkillGraphCluster {
  clusterId: string;
  name: string;
  summary: string;
  memberSkillIds: string[];
  coreSkillIds: string[];
  peripheral: boolean;
}

export interface SkillGraphProximity {
  sourceSkillId: string;
  targetSkillId: string;
  weight: number;
  relationshipTypes: RelationshipType[];
}

export interface SkillGraphEdge {
  edgeId: string;
  sourceSkillId: string;
  targetSkillId: string;
  similarity: number;
  relations: SkillGraphRelation[];
  nearestFallback: boolean;
}

export interface SkillGraphRelation {
  relationshipType: RelationshipType;
  vectorType?: VectorType;
  state: string;
  score?: number;
  source: "stored" | "vector_similarity";
  evidence: unknown;
}

export interface ValidationSample {
  sampleId: string;
  datasetId: string;
  datasetSchemaVersion: string;
  datasetContentVersion: string;
  split: "tuning" | "validation" | "holdout";
  skillId?: string;
  chunkId?: string;
  queryText: string;
  queryLanguage: string;
  taskType: string;
  label: string;
  state: string;
  evidence: unknown;
  confidence?: number;
  profileId?: string;
  inputHash: string;
  createdAt: number;
  reviewedAt?: number;
}

export interface ValidationRun {
  runId: string;
  datasetId: string;
  datasetContentVersion: string;
  profileId: string;
  status: string;
  metrics: Record<string, unknown>;
  startedAt: number;
  completedAt?: number;
  error?: string;
}

export interface LocalValidationExport {
  schemaVersion: string;
  localOnly: true;
  externalWritePerformed: false;
  profileId?: string;
  exportedAt: number;
  samples: ValidationSample[];
  runs: ValidationRun[];
  feedbackEvents: FeedbackEvent[];
}

export interface LocalValidationMutation {
  samples: number;
  runs: number;
  feedbackEvents: number;
  relations: number;
}

export type FeedbackAction =
  | "move_closer"
  | "move_farther"
  | "must_link"
  | "cannot_link"
  | "rank_above"
  | "confirm_relation"
  | "reject_relation"
  | "confirm_no_result"
  | "forbid_trigger";

export interface FeedbackRequest {
  datasetId?: string;
  skillId?: string;
  queryId?: string;
  queryText?: string;
  otherSkillId?: string;
  relationType?: RelationshipType;
  action: FeedbackAction;
  strength?: number;
  profileId?: string;
  sourceEvidence?: unknown;
}

export interface FeedbackEvent {
  feedbackId: string;
  datasetId: string;
  skillId?: string;
  queryId?: string;
  otherSkillId?: string;
  relationType?: RelationshipType;
  action: FeedbackAction;
  strength?: number;
  beforeState: unknown;
  afterState: unknown;
  profileId?: string;
  sourceEvidence: unknown;
  confirmedAt: number;
  revertedAt?: number;
}
