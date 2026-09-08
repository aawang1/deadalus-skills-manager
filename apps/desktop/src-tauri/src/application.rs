use crate::vectorization::{
    RelationshipState, RelationshipType, SearchEvidence, SearchResult, SkillRelationship,
    StoredVector, VectorLevel, VectorStatus, VectorType,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRequest {
    pub skill_id: String,
    pub input_hash: String,
    pub markdown: String,
    pub schema_version: String,
    pub phase: AnalysisPhase,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisPhase {
    StructureExtraction,
    ConflictResolution,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisEvidence {
    pub source_file: String,
    pub heading_path: Option<String>,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuleAnalysisResult {
    pub result_id: String,
    pub skill_id: String,
    pub parser_version: String,
    pub input_hash: String,
    pub fields: BTreeMap<String, serde_json::Value>,
    pub evidence: Vec<AnalysisEvidence>,
    pub confidence: f32,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LlmAnalysisResult {
    pub result_id: String,
    pub skill_id: String,
    pub provider: String,
    pub model: String,
    pub prompt_version: String,
    pub prompt: String,
    pub schema_version: String,
    pub input_hash: String,
    pub fields: BTreeMap<String, serde_json::Value>,
    pub evidence: Vec<AnalysisEvidence>,
    pub confidence: f32,
    pub raw_output: serde_json::Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonStatus {
    Consistent,
    Complementary,
    Conflict,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisComparison {
    pub comparison_id: String,
    pub rule_result_id: String,
    pub llm_result_id: String,
    pub status: ComparisonStatus,
    pub differing_fields: Vec<String>,
    pub adopted_fields: BTreeMap<String, serde_json::Value>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConflictResolutionResult {
    pub resolution_id: String,
    pub comparison_id: String,
    pub provider: String,
    pub model: String,
    pub prompt_version: String,
    pub prompt: String,
    pub complete_input: serde_json::Value,
    pub output: serde_json::Value,
    pub status: ConflictResolutionStatus,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolutionStatus {
    Resolved,
    StillConflicted,
    NeedsReview,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SemanticRecordStatus {
    Ready,
    Expired,
    Failed,
    Pending,
}

impl SemanticRecordStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Expired => "expired",
            Self::Failed => "failed",
            Self::Pending => "pending",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillClassification {
    pub profile_id: String,
    pub skill_id: String,
    pub input_hash: String,
    pub schema_version: String,
    pub provider: String,
    pub model: String,
    pub prompt_version: String,
    pub broad_category: String,
    pub small_categories: Vec<String>,
    pub target_object: String,
    pub user_goal: String,
    pub capability_summary: String,
    pub workflow_summary: String,
    pub confidence: f32,
    pub evidence: Vec<AnalysisEvidence>,
    pub status: SemanticRecordStatus,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSemantic {
    pub profile_id: String,
    pub cluster_id: String,
    pub member_hash: String,
    pub schema_version: String,
    pub provider: String,
    pub model: String,
    pub prompt_version: String,
    pub name: String,
    pub summary: String,
    pub status: SemanticRecordStatus,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Error)]
pub enum AnalysisProviderError {
    #[error("analysis provider is not configured")]
    NotConfigured,
    #[error("analysis provider rejected the request: {0}")]
    Rejected(String),
    #[error("analysis provider returned invalid output: {0}")]
    InvalidOutput(String),
}

#[async_trait]
pub trait AnalysisProvider: Send + Sync {
    async fn analyze(
        &self,
        request: AnalysisRequest,
    ) -> Result<LlmAnalysisResult, AnalysisProviderError>;

    async fn resolve_conflict(
        &self,
        request: AnalysisRequest,
        rule_result: &RuleAnalysisResult,
        llm_result: &LlmAnalysisResult,
    ) -> Result<ConflictResolutionResult, AnalysisProviderError>;
}

#[derive(Debug, Clone)]
pub struct RetrievalConfig {
    pub parent_pool_min: usize,
    pub parent_pool_max: usize,
    pub parent_min_score: f32,
    pub supplementary_chunks_per_type: usize,
    pub supplementary_chunk_min_score: f32,
    pub supplementary_parent_min_score: f32,
    pub chunks_per_skill: usize,
    pub chunk_weight: f32,
    pub expired_penalty: f32,
    pub max_final_results: usize,
    pub min_final_results: usize,
    pub final_min_score: f32,
    pub score_gap: f32,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            parent_pool_min: 4,
            parent_pool_max: 32,
            parent_min_score: 0.25,
            supplementary_chunks_per_type: 8,
            supplementary_chunk_min_score: 0.6,
            supplementary_parent_min_score: 0.15,
            chunks_per_skill: 3,
            chunk_weight: 0.3,
            expired_penalty: 0.85,
            max_final_results: 12,
            min_final_results: 0,
            final_min_score: 0.25,
            score_gap: 0.25,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueryVectors {
    pub profile_id: String,
    pub vectors: BTreeMap<VectorType, Vec<f32>>,
}

#[derive(Debug, Error, PartialEq)]
pub enum RetrievalError {
    #[error("vectors have different dimensions: {left} and {right}")]
    DimensionMismatch { left: usize, right: usize },
    #[error("cosine similarity is undefined for a zero vector")]
    ZeroVector,
}

pub fn exact_cosine(left: &[f32], right: &[f32]) -> Result<f32, RetrievalError> {
    if left.len() != right.len() {
        return Err(RetrievalError::DimensionMismatch {
            left: left.len(),
            right: right.len(),
        });
    }
    let mut dot = 0.0f64;
    let mut left_norm = 0.0f64;
    let mut right_norm = 0.0f64;
    for (&left_value, &right_value) in left.iter().zip(right) {
        let left_value = f64::from(left_value);
        let right_value = f64::from(right_value);
        dot += left_value * right_value;
        left_norm += left_value * left_value;
        right_norm += right_value * right_value;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return Err(RetrievalError::ZeroVector);
    }
    Ok((dot / (left_norm.sqrt() * right_norm.sqrt())) as f32)
}

#[derive(Clone, Copy)]
struct ScoredVector<'a> {
    vector: &'a StoredVector,
    raw_score: f32,
    adjusted_score: f32,
}

#[derive(Default)]
struct SkillAccumulator<'a> {
    parents: Vec<ScoredVector<'a>>,
    chunks: Vec<ScoredVector<'a>>,
}

pub fn retrieve_parent_first(
    query: &QueryVectors,
    stored_vectors: &[StoredVector],
    config: &RetrievalConfig,
) -> Result<Vec<SearchResult>, RetrievalError> {
    let selected = select_fresh_or_expired(query, stored_vectors);
    let mut parent_scores: HashMap<(String, VectorType), ScoredVector<'_>> = HashMap::new();
    let mut parent_pools: BTreeMap<VectorType, Vec<ScoredVector<'_>>> = BTreeMap::new();

    for vector in &selected {
        if vector.level != VectorLevel::Parent {
            continue;
        }
        let Some(query_vector) = query.vectors.get(&vector.vector_type) else {
            continue;
        };
        let raw_score = exact_cosine(query_vector, &vector.vector)?;
        let score = scored(vector, raw_score, config.expired_penalty);
        let key = (vector.skill_id.clone(), vector.vector_type);
        if parent_scores
            .get(&key)
            .is_none_or(|existing| score.adjusted_score > existing.adjusted_score)
        {
            parent_scores.insert(key, score);
        }
    }
    for score in parent_scores.values().copied() {
        parent_pools
            .entry(score.vector.vector_type)
            .or_default()
            .push(score);
    }

    let distinct_skill_count = parent_scores
        .keys()
        .map(|(skill_id, _)| skill_id)
        .collect::<HashSet<_>>()
        .len();
    let dynamic_parent_k = dynamic_pool_size(distinct_skill_count, config);
    let mut accumulators: HashMap<String, SkillAccumulator<'_>> = HashMap::new();

    for pool in parent_pools.values_mut() {
        sort_scores(pool);
        for score in pool
            .iter()
            .filter(|score| score.adjusted_score >= config.parent_min_score)
            .take(dynamic_parent_k)
            .copied()
        {
            accumulators
                .entry(score.vector.skill_id.clone())
                .or_default()
                .parents
                .push(score);
        }
    }

    let mut chunk_pools: BTreeMap<VectorType, Vec<ScoredVector<'_>>> = BTreeMap::new();
    for vector in &selected {
        if vector.level != VectorLevel::Chunk {
            continue;
        }
        let Some(query_vector) = query.vectors.get(&vector.vector_type) else {
            continue;
        };
        let raw_score = exact_cosine(query_vector, &vector.vector)?;
        let score = scored(vector, raw_score, config.expired_penalty);
        chunk_pools
            .entry(vector.vector_type)
            .or_default()
            .push(score);
    }

    // Parent candidates search all of their chunks. Only the strongest bounded evidence
    // contributes later, so a large Skill does not receive an additive count advantage.
    for pool in chunk_pools.values() {
        for score in pool {
            if let Some(accumulator) = accumulators.get_mut(&score.vector.skill_id) {
                accumulator.chunks.push(*score);
            }
        }
    }

    // The supplementary channel is independently bounded per vector type. A strong
    // chunk can rescue a Skill only when its corresponding Parent has basic support.
    for pool in chunk_pools.values_mut() {
        sort_scores(pool);
        for score in pool
            .iter()
            .filter(|score| score.adjusted_score >= config.supplementary_chunk_min_score)
            .take(config.supplementary_chunks_per_type)
            .copied()
        {
            let parent_key = (score.vector.skill_id.clone(), score.vector.vector_type);
            let Some(parent) = parent_scores.get(&parent_key).copied() else {
                continue;
            };
            if parent.adjusted_score < config.supplementary_parent_min_score {
                continue;
            }
            let accumulator = accumulators
                .entry(score.vector.skill_id.clone())
                .or_default();
            if !accumulator
                .parents
                .iter()
                .any(|candidate| candidate.vector.embedding_id == parent.vector.embedding_id)
            {
                accumulator.parents.push(parent);
            }
            if !accumulator
                .chunks
                .iter()
                .any(|candidate| candidate.vector.embedding_id == score.vector.embedding_id)
            {
                accumulator.chunks.push(score);
            }
        }
    }

    let mut results = accumulators
        .into_iter()
        .filter_map(|(skill_id, mut accumulator)| {
            if accumulator.parents.is_empty() {
                return None;
            }
            sort_scores(&mut accumulator.parents);
            sort_scores(&mut accumulator.chunks);
            accumulator.chunks.truncate(config.chunks_per_skill);
            let parent_score = accumulator
                .parents
                .iter()
                .map(|score| score.adjusted_score)
                .fold(f32::NEG_INFINITY, f32::max);
            let chunk_score = (!accumulator.chunks.is_empty()).then(|| {
                accumulator
                    .chunks
                    .iter()
                    .map(|score| score.adjusted_score)
                    .sum::<f32>()
                    / accumulator.chunks.len() as f32
            });
            let score = chunk_score.map_or(parent_score, |chunk_score| {
                parent_score * (1.0 - config.chunk_weight) + chunk_score * config.chunk_weight
            });
            if score < config.final_min_score {
                return None;
            }
            let matched_types = accumulator
                .parents
                .iter()
                .chain(&accumulator.chunks)
                .map(|score| score.vector.vector_type)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let evidence = accumulator
                .parents
                .iter()
                .chain(&accumulator.chunks)
                .map(|score| evidence(score))
                .collect::<Vec<_>>();
            let expired = evidence.iter().any(|item| item.expired);
            Some(SearchResult {
                skill_id,
                score,
                parent_score,
                chunk_score,
                expired,
                matched_types,
                evidence,
            })
        })
        .collect::<Vec<_>>();
    results.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.skill_id.cmp(&right.skill_id))
    });

    let mut bounded: Vec<SearchResult> = Vec::new();
    for result in results {
        if bounded.len() >= config.max_final_results {
            break;
        }
        if bounded.len() >= config.min_final_results {
            if let Some(previous) = bounded.last() {
                if previous.score - result.score >= config.score_gap {
                    break;
                }
            }
        }
        bounded.push(result);
    }
    Ok(bounded)
}

fn select_fresh_or_expired<'a>(
    query: &QueryVectors,
    vectors: &'a [StoredVector],
) -> Vec<&'a StoredVector> {
    let mut selected: HashMap<(String, VectorType, VectorLevel, String), &StoredVector> =
        HashMap::new();
    for vector in vectors {
        if vector.profile_id != query.profile_id
            || !matches!(vector.status, VectorStatus::Ready | VectorStatus::Expired)
        {
            continue;
        }
        let logical_id = vector
            .chunk_id
            .clone()
            .or_else(|| vector.resource_category.clone())
            .unwrap_or_else(|| "parent".to_string());
        let key = (
            vector.skill_id.clone(),
            vector.vector_type,
            vector.level,
            logical_id,
        );
        match selected.get(&key) {
            None => {
                selected.insert(key, vector);
            }
            Some(existing)
                if existing.status == VectorStatus::Expired
                    && vector.status == VectorStatus::Ready =>
            {
                selected.insert(key, vector);
            }
            Some(existing)
                if existing.status == vector.status
                    && vector.embedding_id < existing.embedding_id =>
            {
                selected.insert(key, vector);
            }
            _ => {}
        }
    }
    let mut values = selected.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| left.embedding_id.cmp(&right.embedding_id));
    values
}

fn scored<'a>(vector: &'a StoredVector, raw_score: f32, expired_penalty: f32) -> ScoredVector<'a> {
    ScoredVector {
        vector,
        raw_score,
        adjusted_score: if vector.status == VectorStatus::Expired {
            raw_score * expired_penalty
        } else {
            raw_score
        },
    }
}

fn evidence(score: &ScoredVector<'_>) -> SearchEvidence {
    SearchEvidence {
        embedding_id: score.vector.embedding_id.clone(),
        vector_type: score.vector.vector_type,
        level: score.vector.level,
        chunk_id: score.vector.chunk_id.clone(),
        raw_score: score.raw_score,
        expired: score.vector.status == VectorStatus::Expired,
        heading_path: score.vector.heading_path.clone(),
        source_file: score.vector.source_file.clone(),
    }
}

fn sort_scores(scores: &mut [ScoredVector<'_>]) {
    scores.sort_by(|left, right| {
        right
            .adjusted_score
            .total_cmp(&left.adjusted_score)
            .then_with(|| left.vector.embedding_id.cmp(&right.vector.embedding_id))
    });
}

fn dynamic_pool_size(skill_count: usize, config: &RetrievalConfig) -> usize {
    if skill_count == 0 {
        return 0;
    }
    let root = (skill_count as f64).sqrt().ceil() as usize;
    (root * 2)
        .max(config.parent_pool_min)
        .min(config.parent_pool_max.max(config.parent_pool_min))
        .min(skill_count)
}

pub fn retrieve_semantic_results(
    query: &QueryVectors,
    stored_vectors: &[StoredVector],
    allowed_skill_ids: Option<&HashSet<String>>,
    config: &RetrievalConfig,
) -> Result<Vec<SearchResult>, RetrievalError> {
    let filtered = stored_vectors
        .iter()
        .filter(|vector| vector.profile_id == query.profile_id)
        .filter(|vector| allowed_skill_ids.is_none_or(|allowed| allowed.contains(&vector.skill_id)))
        .cloned()
        .collect::<Vec<_>>();
    retrieve_parent_first(query, &filtered, config)
}

pub fn migrate_profile_relations(
    source_relations: &[SkillRelationship],
    target_vectors: &[StoredVector],
    source_profile_id: &str,
    target_profile_id: &str,
    max_neighbors: usize,
    now: i64,
) -> Vec<SkillRelationship> {
    let mut output = Vec::new();
    let mut hints = HashMap::new();
    for relationship in source_relations {
        if relationship.state == RelationshipState::HumanReverted {
            continue;
        }
        let (source, target) =
            ordered_pair(&relationship.source_skill_id, &relationship.target_skill_id);
        if matches!(
            relationship.state,
            RelationshipState::CarriedFact
                | RelationshipState::HumanConfirmed
                | RelationshipState::HumanRejected
        ) {
            let mut copied = relationship.clone();
            copied.relation_id = relation_id(
                "carried",
                target_profile_id,
                source,
                target,
                relationship.vector_type,
            );
            copied.source_profile_id = Some(source_profile_id.to_string());
            copied.target_profile_id = Some(target_profile_id.to_string());
            copied.created_at = now;
            output.push(copied);
        } else {
            let mut hint = relationship.clone();
            hint.relation_id = relation_id(
                "hint",
                target_profile_id,
                source,
                target,
                relationship.vector_type,
            );
            hint.source_profile_id = Some(source_profile_id.to_string());
            hint.target_profile_id = Some(target_profile_id.to_string());
            hint.state = RelationshipState::MigrationHint;
            hint.evidence = serde_json::json!({
                "sourceRelationId": relationship.relation_id,
                "sourceState": relationship.state.as_str(),
                "sourceEvidence": relationship.evidence,
            });
            hint.created_at = now;
            hint.validated_at = None;
            hints.insert(
                (
                    source.to_string(),
                    target.to_string(),
                    relationship.vector_type,
                ),
                hint.clone(),
            );
            output.push(hint);
        }
    }

    let parents = target_vectors
        .iter()
        .filter(|vector| {
            vector.profile_id == target_profile_id
                && vector.level == VectorLevel::Parent
                && vector.status == VectorStatus::Ready
        })
        .collect::<Vec<_>>();
    let mut seen_pairs = HashSet::new();
    for source in &parents {
        let mut neighbors = parents
            .iter()
            .filter(|target| {
                source.skill_id != target.skill_id && source.vector_type == target.vector_type
            })
            .filter_map(|target| {
                exact_cosine(&source.vector, &target.vector)
                    .ok()
                    .map(|score| (*target, score))
            })
            .filter(|(_, score)| *score >= 0.6)
            .collect::<Vec<_>>();
        neighbors.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.skill_id.cmp(&right.0.skill_id))
        });
        for (target, score) in neighbors.into_iter().take(max_neighbors) {
            let (left, right) = ordered_pair(&source.skill_id, &target.skill_id);
            let key = (
                left.to_string(),
                right.to_string(),
                Some(source.vector_type),
            );
            if !seen_pairs.insert(key.clone()) {
                continue;
            }
            let hint = hints.get(&key);
            let state = if hint.is_some_and(|hint| {
                hint.relationship_type == RelationshipType::ConflictsWith && score >= 0.8
            }) {
                RelationshipState::Conflict
            } else {
                RelationshipState::Revalidated
            };
            output.push(SkillRelationship {
                relation_id: relation_id(
                    "neighbor",
                    target_profile_id,
                    left,
                    right,
                    Some(source.vector_type),
                ),
                source_skill_id: left.to_string(),
                target_skill_id: right.to_string(),
                relationship_type: hint
                    .map(|hint| hint.relationship_type)
                    .unwrap_or(RelationshipType::SimilarTo),
                vector_type: Some(source.vector_type),
                source_profile_id: Some(target_profile_id.to_string()),
                target_profile_id: Some(target_profile_id.to_string()),
                score: Some(f64::from(score)),
                state,
                evidence: serde_json::json!({
                    "method": "independent_nearest_neighbor",
                    "sourceEmbeddingId": source.embedding_id,
                    "targetEmbeddingId": target.embedding_id,
                    "migrationHintId": hint.map(|hint| hint.relation_id.as_str()),
                }),
                created_at: now,
                validated_at: Some(now),
            });
        }
    }
    for (key, hint) in hints {
        if !seen_pairs.contains(&key) {
            output.push(SkillRelationship {
                relation_id: relation_id("rejected", target_profile_id, &key.0, &key.1, key.2),
                source_skill_id: key.0,
                target_skill_id: key.1,
                relationship_type: hint.relationship_type,
                vector_type: key.2,
                source_profile_id: Some(source_profile_id.to_string()),
                target_profile_id: Some(target_profile_id.to_string()),
                score: None,
                state: RelationshipState::Rejected,
                evidence: serde_json::json!({
                    "method": "independent_nearest_neighbor",
                    "migrationHintId": hint.relation_id,
                    "reason": "not_discovered_in_bounded_target_neighbors",
                }),
                created_at: now,
                validated_at: Some(now),
            });
        }
    }
    output.sort_by(|left, right| left.relation_id.cmp(&right.relation_id));
    output
}

fn ordered_pair<'a>(left: &'a str, right: &'a str) -> (&'a str, &'a str) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn relation_id(
    prefix: &str,
    profile_id: &str,
    source: &str,
    target: &str,
    vector_type: Option<VectorType>,
) -> String {
    format!(
        "{prefix}:{profile_id}:{source}:{target}:{}",
        vector_type.map_or("none", VectorType::as_str)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector(
        id: &str,
        skill: &str,
        vector_type: VectorType,
        level: VectorLevel,
        values: &[f32],
        status: VectorStatus,
    ) -> StoredVector {
        StoredVector {
            embedding_id: id.to_string(),
            profile_id: "profile".to_string(),
            skill_id: skill.to_string(),
            vector_type,
            level,
            parent_embedding_id: None,
            resource_category: None,
            chunk_id: (level == VectorLevel::Chunk).then(|| format!("{id}-chunk")),
            input_hash: id.to_string(),
            vector: values.to_vec(),
            status,
            heading_path: None,
            source_file: None,
        }
    }

    #[test]
    fn cosine_is_exact_and_rejects_invalid_vectors() {
        assert!((exact_cosine(&[1.0, 2.0], &[1.0, 2.0]).unwrap() - 1.0).abs() < 1e-6);
        assert_eq!(exact_cosine(&[1.0, 0.0], &[0.0, 1.0]).unwrap(), 0.0);
        assert_eq!(exact_cosine(&[1.0, 0.0], &[-1.0, 0.0]).unwrap(), -1.0);
        assert_eq!(
            exact_cosine(&[1.0], &[1.0, 2.0]),
            Err(RetrievalError::DimensionMismatch { left: 1, right: 2 })
        );
        assert_eq!(
            exact_cosine(&[0.0, 0.0], &[1.0, 2.0]),
            Err(RetrievalError::ZeroVector)
        );
    }

    #[test]
    fn retrieval_uses_parent_pools_supplementary_chunks_and_skill_deduplication() {
        let mut queries = BTreeMap::new();
        queries.insert(VectorType::Workflow, vec![1.0, 0.0]);
        let query = QueryVectors {
            profile_id: "profile".to_string(),
            vectors: queries,
        };
        let vectors = vec![
            vector(
                "a-parent",
                "skill-a",
                VectorType::Workflow,
                VectorLevel::Parent,
                &[1.0, 0.0],
                VectorStatus::Ready,
            ),
            vector(
                "a-chunk",
                "skill-a",
                VectorType::Workflow,
                VectorLevel::Chunk,
                &[0.9, 0.1],
                VectorStatus::Ready,
            ),
            vector(
                "b-parent",
                "skill-b",
                VectorType::Workflow,
                VectorLevel::Parent,
                &[0.2, 0.98],
                VectorStatus::Ready,
            ),
            vector(
                "b-chunk-one",
                "skill-b",
                VectorType::Workflow,
                VectorLevel::Chunk,
                &[1.0, 0.0],
                VectorStatus::Ready,
            ),
            vector(
                "b-chunk-two",
                "skill-b",
                VectorType::Workflow,
                VectorLevel::Chunk,
                &[0.95, 0.05],
                VectorStatus::Ready,
            ),
        ];
        let config = RetrievalConfig {
            parent_pool_min: 1,
            parent_pool_max: 1,
            parent_min_score: 0.8,
            supplementary_chunk_min_score: 0.9,
            supplementary_parent_min_score: 0.15,
            final_min_score: 0.1,
            score_gap: 1.0,
            ..Default::default()
        };
        let results = retrieve_parent_first(&query, &vectors, &config).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(
            results
                .iter()
                .filter(|result| result.skill_id == "skill-b")
                .count(),
            1
        );
        assert!(results
            .iter()
            .find(|result| result.skill_id == "skill-b")
            .unwrap()
            .chunk_score
            .is_some());
    }

    #[test]
    fn ready_vectors_replace_expired_versions_and_expired_fallback_is_marked() {
        let query = QueryVectors {
            profile_id: "profile".to_string(),
            vectors: BTreeMap::from([(VectorType::Trigger, vec![1.0, 0.0])]),
        };
        let mut expired_old = vector(
            "a-expired",
            "skill-a",
            VectorType::Trigger,
            VectorLevel::Parent,
            &[1.0, 0.0],
            VectorStatus::Expired,
        );
        expired_old.resource_category = Some("same-logical-parent".to_string());
        let mut ready = vector(
            "a-ready",
            "skill-a",
            VectorType::Trigger,
            VectorLevel::Parent,
            &[1.0, 0.0],
            VectorStatus::Ready,
        );
        ready.resource_category = Some("same-logical-parent".to_string());
        let expired_only = vector(
            "b-expired",
            "skill-b",
            VectorType::Trigger,
            VectorLevel::Parent,
            &[0.9, 0.1],
            VectorStatus::Expired,
        );
        let config = RetrievalConfig {
            parent_min_score: 0.1,
            final_min_score: 0.1,
            score_gap: 1.0,
            ..Default::default()
        };
        let results =
            retrieve_parent_first(&query, &[expired_old, ready, expired_only], &config).unwrap();
        let skill_a = results
            .iter()
            .find(|result| result.skill_id == "skill-a")
            .unwrap();
        let skill_b = results
            .iter()
            .find(|result| result.skill_id == "skill-b")
            .unwrap();
        assert!(!skill_a.expired);
        assert!(skill_b.expired);
        assert!(skill_a.score > skill_b.score);
    }

    #[test]
    fn semantic_retrieval_filters_membership_profile_and_allows_zero_results() {
        let query = QueryVectors {
            profile_id: "profile".to_string(),
            vectors: BTreeMap::from([(VectorType::Workflow, vec![1.0, 0.0])]),
        };
        let allowed = HashSet::from(["skill-a".to_string()]);
        let mut cross_profile = vector(
            "cross",
            "skill-cross",
            VectorType::Workflow,
            VectorLevel::Parent,
            &[1.0, 0.0],
            VectorStatus::Ready,
        );
        cross_profile.profile_id = "other-profile".to_string();
        let stored = vec![
            vector(
                "allowed",
                "skill-a",
                VectorType::Workflow,
                VectorLevel::Parent,
                &[1.0, 0.0],
                VectorStatus::Ready,
            ),
            vector(
                "blocked",
                "skill-b",
                VectorType::Workflow,
                VectorLevel::Parent,
                &[1.0, 0.0],
                VectorStatus::Ready,
            ),
            cross_profile,
        ];
        let config = RetrievalConfig {
            parent_min_score: 0.5,
            final_min_score: 0.5,
            ..Default::default()
        };
        let results = retrieve_semantic_results(&query, &stored, Some(&allowed), &config).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].skill_id, "skill-a");

        let zero_query = QueryVectors {
            profile_id: "profile".to_string(),
            vectors: BTreeMap::from([(VectorType::Workflow, vec![0.0, 1.0])]),
        };
        assert!(
            retrieve_semantic_results(&zero_query, &stored, Some(&allowed), &config)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn migration_carries_facts_rejects_stale_hints_and_discovers_independently() {
        let relation = |id: &str,
                        source: &str,
                        target: &str,
                        relationship_type: RelationshipType,
                        state: RelationshipState| SkillRelationship {
            relation_id: id.to_string(),
            source_skill_id: source.to_string(),
            target_skill_id: target.to_string(),
            relationship_type,
            vector_type: Some(VectorType::Workflow),
            source_profile_id: Some("old".to_string()),
            target_profile_id: Some("old".to_string()),
            score: Some(0.9),
            state,
            evidence: serde_json::json!({"old": true}),
            created_at: 1,
            validated_at: None,
        };
        let old = vec![
            relation(
                "fact",
                "skill-a",
                "skill-d",
                RelationshipType::DependsOn,
                RelationshipState::HumanConfirmed,
            ),
            relation(
                "stale",
                "skill-a",
                "skill-b",
                RelationshipType::SimilarTo,
                RelationshipState::Revalidated,
            ),
            relation(
                "conflict",
                "skill-a",
                "skill-c",
                RelationshipType::ConflictsWith,
                RelationshipState::Revalidated,
            ),
        ];
        let mut vectors = vec![
            vector(
                "a",
                "skill-a",
                VectorType::Workflow,
                VectorLevel::Parent,
                &[1.0, 0.0],
                VectorStatus::Ready,
            ),
            vector(
                "b",
                "skill-b",
                VectorType::Workflow,
                VectorLevel::Parent,
                &[0.0, 1.0],
                VectorStatus::Ready,
            ),
            vector(
                "c",
                "skill-c",
                VectorType::Workflow,
                VectorLevel::Parent,
                &[0.99, 0.01],
                VectorStatus::Ready,
            ),
        ];
        for vector in &mut vectors {
            vector.profile_id = "target".to_string();
        }
        let migrated = migrate_profile_relations(&old, &vectors, "old", "target", 2, 10);
        assert!(migrated
            .iter()
            .any(|relation| relation.state == RelationshipState::HumanConfirmed));
        assert!(migrated
            .iter()
            .any(|relation| relation.state == RelationshipState::MigrationHint));
        assert!(migrated
            .iter()
            .any(|relation| relation.state == RelationshipState::Rejected
                && relation.target_skill_id == "skill-b"));
        assert!(migrated
            .iter()
            .any(|relation| relation.state == RelationshipState::Conflict
                && relation.target_skill_id == "skill-c"));
    }
}
