use crate::database::{CanonicalSnapshot, ValidationSampleRecord};
use crate::pipeline::prepare_skill_inputs;
use crate::vectorization::{
    stable_hash, RelationshipType, SearchResult, ValidationState, VectorType,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const LOCAL_DATASET_SCHEMA_VERSION: &str = "deadalus.local-validation.v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationTaskType {
    OverallFunction,
    Trigger,
    Workflow,
    Resource,
    ChunkEvidence,
    NoResult,
    Forbidden,
}

impl ValidationTaskType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OverallFunction => "overall_function",
            Self::Trigger => "trigger",
            Self::Workflow => "workflow",
            Self::Resource => "resource",
            Self::ChunkEvidence => "chunk_evidence",
            Self::NoResult => "no_result",
            Self::Forbidden => "forbidden",
        }
    }

    pub const fn vector_type(self) -> Option<VectorType> {
        match self {
            Self::OverallFunction => Some(VectorType::OverallFunction),
            Self::Trigger | Self::Forbidden => Some(VectorType::Trigger),
            Self::Workflow => Some(VectorType::Workflow),
            Self::Resource => Some(VectorType::Resource),
            Self::ChunkEvidence | Self::NoResult => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackAction {
    MoveCloser,
    MoveFarther,
    MustLink,
    CannotLink,
    RankAbove,
    ConfirmRelation,
    RejectRelation,
    ConfirmNoResult,
    ForbidTrigger,
}

impl FeedbackAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MoveCloser => "move_closer",
            Self::MoveFarther => "move_farther",
            Self::MustLink => "must_link",
            Self::CannotLink => "cannot_link",
            Self::RankAbove => "rank_above",
            Self::ConfirmRelation => "confirm_relation",
            Self::RejectRelation => "reject_relation",
            Self::ConfirmNoResult => "confirm_no_result",
            Self::ForbidTrigger => "forbid_trigger",
        }
    }

    pub const fn confirms(self) -> bool {
        matches!(
            self,
            Self::MoveCloser
                | Self::MustLink
                | Self::RankAbove
                | Self::ConfirmRelation
                | Self::ConfirmNoResult
                | Self::ForbidTrigger
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeedbackRequest {
    pub dataset_id: Option<String>,
    pub skill_id: Option<String>,
    pub query_id: Option<String>,
    pub query_text: Option<String>,
    pub other_skill_id: Option<String>,
    pub relation_type: Option<RelationshipType>,
    pub action: FeedbackAction,
    pub strength: Option<f64>,
    pub profile_id: Option<String>,
    #[serde(default)]
    pub source_evidence: Value,
}

impl FeedbackRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .strength
            .is_some_and(|strength| !strength.is_finite() || !(0.0..=1.0).contains(&strength))
        {
            return Err("strength must be a finite value between 0 and 1".to_string());
        }
        let has_skill = self.skill_id.as_deref().is_some_and(non_empty);
        let has_other = self.other_skill_id.as_deref().is_some_and(non_empty);
        let has_query = self.query_id.as_deref().is_some_and(non_empty)
            || self.query_text.as_deref().is_some_and(non_empty);
        match self.action {
            FeedbackAction::MoveCloser
            | FeedbackAction::MoveFarther
            | FeedbackAction::MustLink
            | FeedbackAction::CannotLink
            | FeedbackAction::ConfirmRelation
            | FeedbackAction::RejectRelation => {
                if !has_skill || !has_other || self.relation_type.is_none() {
                    return Err(
                        "relation feedback requires skillId, otherSkillId, and relationType"
                            .to_string(),
                    );
                }
            }
            FeedbackAction::RankAbove => {
                if !has_skill || !has_other || !has_query {
                    return Err(
                        "rank_above requires skillId, otherSkillId, and queryId/queryText"
                            .to_string(),
                    );
                }
            }
            FeedbackAction::ConfirmNoResult => {
                if !has_query {
                    return Err("confirm_no_result requires queryId or queryText".to_string());
                }
            }
            FeedbackAction::ForbidTrigger => {
                if !has_skill || !has_query {
                    return Err("forbid_trigger requires skillId and queryId/queryText".to_string());
                }
            }
        }
        if has_skill && self.skill_id == self.other_skill_id {
            return Err("semantic feedback cannot relate a Skill to itself".to_string());
        }
        Ok(())
    }
}

fn non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedAnalysisRecord {
    pub skill_id: String,
    pub rule_result: Option<Value>,
    pub comparison_status: Option<String>,
    pub llm_result: Option<Value>,
}

#[derive(Debug, Clone)]
struct SampleCandidate {
    family_id: String,
    skill_id: Option<String>,
    chunk_id: Option<String>,
    query_text: String,
    task_type: ValidationTaskType,
    label: String,
    state: ValidationState,
    evidence: Value,
    confidence: Option<f64>,
    input_hash: String,
}

pub fn generate_samples(
    snapshot: &CanonicalSnapshot,
    analyses: &[SavedAnalysisRecord],
    profile_id: Option<&str>,
    created_at: i64,
) -> Vec<ValidationSampleRecord> {
    let analysis_by_skill = analyses
        .iter()
        .map(|analysis| (analysis.skill_id.as_str(), analysis))
        .collect::<HashMap<_, _>>();
    let mut candidates = Vec::new();
    for skill in &snapshot.skills {
        let Ok(prepared) = prepare_skill_inputs(skill, &Default::default()) else {
            continue;
        };
        for input in &prepared.inputs {
            let task_type = task_for_vector(input.parent.vector_type);
            let query_text = query_from_input(&input.parent.title, &input.parent.fields);
            if !query_text.is_empty() {
                candidates.push(SampleCandidate {
                    family_id: skill.skill_id.clone(),
                    skill_id: Some(skill.skill_id.clone()),
                    chunk_id: None,
                    query_text: query_text.clone(),
                    task_type,
                    label: "3".to_string(),
                    state: ValidationState::RuleDerived,
                    evidence: json!({
                        "inputId": input.input_id,
                        "inputHash": input.input_hash,
                        "sourceHeadings": input.parent.source_headings,
                        "source": "canonical_structured_input",
                        "savedRuleAnalysis": analysis_by_skill
                            .get(skill.skill_id.as_str())
                            .is_some_and(|analysis| analysis.rule_result.is_some()),
                    }),
                    confidence: Some(0.85),
                    input_hash: input.input_hash.clone(),
                });
                if input.parent.vector_type == VectorType::Trigger
                    && contains_forbidden_language(&input.text)
                {
                    candidates.push(SampleCandidate {
                        family_id: skill.skill_id.clone(),
                        skill_id: Some(skill.skill_id.clone()),
                        chunk_id: None,
                        query_text: query_text.clone(),
                        task_type: ValidationTaskType::Forbidden,
                        label: "X".to_string(),
                        state: ValidationState::RuleDerived,
                        evidence: json!({
                            "inputId": input.input_id,
                            "excerpt": input.text.chars().take(300).collect::<String>(),
                            "source": "deterministic_forbidden_language",
                        }),
                        confidence: Some(0.8),
                        input_hash: input.input_hash.clone(),
                    });
                }
                if let Some(analysis) = analysis_by_skill.get(skill.skill_id.as_str()) {
                    let state = match analysis.comparison_status.as_deref() {
                        Some("consistent") => Some(ValidationState::MachineConsensus),
                        Some("conflict") => Some(ValidationState::Conflict),
                        _ => None,
                    };
                    if let Some(state) = state {
                        candidates.push(SampleCandidate {
                            family_id: skill.skill_id.clone(),
                            skill_id: Some(skill.skill_id.clone()),
                            chunk_id: None,
                            query_text,
                            task_type,
                            label: if state == ValidationState::Conflict {
                                "U".to_string()
                            } else {
                                "3".to_string()
                            },
                            state,
                            evidence: json!({
                                "inputId": input.input_id,
                                "comparisonStatus": analysis.comparison_status,
                                "provisional": true,
                                "gold": false,
                            }),
                            confidence: if state == ValidationState::MachineConsensus {
                                Some(0.65)
                            } else {
                                None
                            },
                            input_hash: input.input_hash.clone(),
                        });
                    }
                }
            }
            for chunk in &input.chunks {
                candidates.push(SampleCandidate {
                    family_id: skill.skill_id.clone(),
                    skill_id: Some(skill.skill_id.clone()),
                    chunk_id: Some(chunk.chunk_id.clone()),
                    query_text: chunk
                        .text
                        .split_once("text:")
                        .map(|(_, text)| text)
                        .unwrap_or(&chunk.text)
                        .chars()
                        .take(180)
                        .collect(),
                    task_type: ValidationTaskType::ChunkEvidence,
                    label: "3".to_string(),
                    state: ValidationState::RuleDerived,
                    evidence: json!({
                        "chunkId": chunk.chunk_id,
                        "headingPath": chunk.heading_path,
                        "sourceFile": chunk.relative_file_path,
                    }),
                    confidence: Some(0.8),
                    input_hash: chunk.input_hash.clone(),
                });
            }
        }
        if let Some(analysis) = analysis_by_skill.get(skill.skill_id.as_str()) {
            add_llm_candidates(skill.skill_id.as_str(), analysis, &mut candidates);
        }
    }
    candidates.sort_by(|left, right| {
        left.family_id
            .cmp(&right.family_id)
            .then_with(|| left.task_type.as_str().cmp(right.task_type.as_str()))
            .then_with(|| left.query_text.cmp(&right.query_text))
            .then_with(|| left.state.as_str().cmp(right.state.as_str()))
    });
    let content_material = candidates
        .iter()
        .map(|candidate| {
            format!(
                "{}\0{}\0{}\0{}\0{}",
                candidate.family_id,
                candidate.task_type.as_str(),
                candidate.query_text,
                candidate.label,
                candidate.state.as_str()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let dataset_id = format!("user-local:{}", snapshot.snapshot_id);
    let content_version = stable_hash(
        format!(
            "{}\0{}\0{}",
            snapshot.content_hash,
            profile_id.unwrap_or("profile-independent"),
            content_material
        )
        .as_bytes(),
    );
    candidates
        .into_iter()
        .map(|candidate| {
            let identity = format!(
                "{}\0{}\0{}\0{}\0{}",
                content_version,
                candidate.family_id,
                candidate.task_type.as_str(),
                candidate.query_text,
                candidate.state.as_str()
            );
            ValidationSampleRecord {
                sample_id: format!("sample_{}", &stable_hash(identity.as_bytes())[..32]),
                dataset_id: dataset_id.clone(),
                dataset_schema_version: LOCAL_DATASET_SCHEMA_VERSION.to_string(),
                dataset_content_version: content_version.clone(),
                split: split_for_family(&candidate.family_id).to_string(),
                skill_id: candidate.skill_id,
                chunk_id: candidate.chunk_id,
                query_text: candidate.query_text.clone(),
                query_language: query_language(&candidate.query_text).to_string(),
                task_type: candidate.task_type.as_str().to_string(),
                label: candidate.label,
                state: candidate.state,
                evidence: candidate.evidence,
                confidence: candidate.confidence,
                profile_id: profile_id.map(str::to_string),
                input_hash: candidate.input_hash,
                created_at,
                reviewed_at: None,
            }
        })
        .collect()
}

fn add_llm_candidates(
    skill_id: &str,
    analysis: &SavedAnalysisRecord,
    candidates: &mut Vec<SampleCandidate>,
) {
    let Some(fields) = analysis
        .llm_result
        .as_ref()
        .and_then(|result| result.get("fields"))
        .and_then(Value::as_object)
    else {
        return;
    };
    for (field, value) in fields {
        let Some(task_type) = task_for_field(field) else {
            continue;
        };
        let values = value
            .as_array()
            .map(|items| items.iter().collect::<Vec<_>>())
            .unwrap_or_else(|| vec![value]);
        for item in values {
            let Some(text) = item.as_str().filter(|text| !text.trim().is_empty()) else {
                continue;
            };
            candidates.push(SampleCandidate {
                family_id: skill_id.to_string(),
                skill_id: (task_type != ValidationTaskType::NoResult).then(|| skill_id.to_string()),
                chunk_id: None,
                query_text: text.trim().chars().take(240).collect(),
                task_type,
                label: if task_type == ValidationTaskType::Forbidden {
                    "X".to_string()
                } else {
                    "2".to_string()
                },
                state: ValidationState::LlmSynthetic,
                evidence: json!({
                    "source": "saved_llm_analysis",
                    "field": field,
                    "provisional": true,
                    "gold": false,
                }),
                confidence: Some(0.45),
                input_hash: stable_hash(text.as_bytes()),
            });
        }
    }
}

fn task_for_field(field: &str) -> Option<ValidationTaskType> {
    match field {
        "overallFunction" | "overall_function" => Some(ValidationTaskType::OverallFunction),
        "trigger" => Some(ValidationTaskType::Trigger),
        "workflow" => Some(ValidationTaskType::Workflow),
        "resource" => Some(ValidationTaskType::Resource),
        "chunkEvidence" | "chunk_evidence" => Some(ValidationTaskType::ChunkEvidence),
        "noResult" | "no_result" => Some(ValidationTaskType::NoResult),
        "forbidden" => Some(ValidationTaskType::Forbidden),
        _ => None,
    }
}

fn task_for_vector(vector_type: VectorType) -> ValidationTaskType {
    match vector_type {
        VectorType::OverallFunction | VectorType::General => ValidationTaskType::OverallFunction,
        VectorType::Trigger => ValidationTaskType::Trigger,
        VectorType::Workflow => ValidationTaskType::Workflow,
        VectorType::Resource => ValidationTaskType::Resource,
    }
}

fn query_from_input(title: &str, fields: &BTreeMap<String, Vec<String>>) -> String {
    let detail = fields
        .iter()
        .filter(|(key, _)| key.as_str() != "name")
        .flat_map(|(_, values)| values)
        .find(|value| !value.trim().is_empty())
        .map(|value| value.trim().chars().take(180).collect::<String>())
        .unwrap_or_default();
    match (title.trim(), detail.as_str()) {
        ("", "") => String::new(),
        ("", detail) => detail.to_string(),
        (title, "") => title.to_string(),
        (title, detail) => format!("{title}: {detail}"),
    }
}

fn contains_forbidden_language(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "do not use",
        "must not",
        "never use",
        "不得",
        "禁止",
        "不要使用",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
}

pub fn split_for_family(family_id: &str) -> &'static str {
    let hash = stable_hash(format!("validation-family\0{family_id}").as_bytes());
    let bucket = u8::from_str_radix(&hash[..2], 16).unwrap_or_default() % 100;
    match bucket {
        0..=59 => "tuning",
        60..=84 => "validation",
        _ => "holdout",
    }
}

fn query_language(value: &str) -> &'static str {
    let has_ascii = value
        .chars()
        .any(|character| character.is_ascii_alphabetic());
    let has_cjk = value
        .chars()
        .any(|character| ('\u{4e00}'..='\u{9fff}').contains(&character));
    match (has_ascii, has_cjk) {
        (true, true) => "mixed",
        (false, true) => "zh",
        _ => "en",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationMetrics {
    pub recall_at_k: Option<f64>,
    pub precision_at_k: Option<f64>,
    pub mrr: Option<f64>,
    pub ndcg: Option<f64>,
    pub no_result_accuracy: Option<f64>,
    pub forbidden_error_rate: Option<f64>,
    pub chunk_evidence_recall: Option<f64>,
    pub evaluated_samples: u64,
    pub no_result_count: u64,
    pub forbidden_count: u64,
    pub evidence_count: u64,
    pub confidence: String,
    pub warnings: Vec<String>,
    pub advisory_only: bool,
    pub blocks_activation: bool,
    pub label_source_counts: BTreeMap<String, u64>,
}

#[derive(Debug, Clone)]
pub struct EvaluatedSample {
    pub sample: ValidationSampleRecord,
    pub results: Vec<SearchResult>,
}

pub fn compute_metrics(evaluated: &[EvaluatedSample], k: usize) -> ValidationMetrics {
    let mut recall = Vec::new();
    let mut precision = Vec::new();
    let mut reciprocal_ranks = Vec::new();
    let mut ndcgs = Vec::new();
    let mut no_result = Vec::new();
    let mut forbidden = Vec::new();
    let mut evidence = Vec::new();
    let mut label_source_counts = BTreeMap::new();
    let mut families = BTreeSet::new();
    for evaluation in evaluated {
        *label_source_counts
            .entry(evaluation.sample.state.as_str().to_string())
            .or_insert(0) += 1;
        if let Some(skill_id) = &evaluation.sample.skill_id {
            families.insert(skill_id.clone());
        }
        let results = evaluation.results.iter().take(k.max(1)).collect::<Vec<_>>();
        match evaluation.sample.task_type.as_str() {
            "no_result" => no_result.push(results.is_empty()),
            "forbidden" => {
                if let Some(skill_id) = &evaluation.sample.skill_id {
                    forbidden.push(results.iter().any(|result| &result.skill_id == skill_id));
                }
            }
            "chunk_evidence" => {
                if let Some(chunk_id) = &evaluation.sample.chunk_id {
                    evidence.push(results.iter().any(|result| {
                        result
                            .evidence
                            .iter()
                            .any(|item| item.chunk_id.as_ref() == Some(chunk_id))
                    }));
                }
            }
            _ if matches!(evaluation.sample.label.as_str(), "1" | "2" | "3") => {
                if let Some(skill_id) = &evaluation.sample.skill_id {
                    let rank = results
                        .iter()
                        .position(|result| &result.skill_id == skill_id);
                    recall.push(rank.is_some());
                    precision.push(if rank.is_some() {
                        1.0 / results.len().max(1) as f64
                    } else {
                        0.0
                    });
                    reciprocal_ranks.push(rank.map_or(0.0, |rank| 1.0 / (rank + 1) as f64));
                    let relevance = evaluation.sample.label.parse::<f64>().unwrap_or_default();
                    ndcgs.push(rank.map_or(0.0, |rank| {
                        ((2.0f64.powf(relevance) - 1.0) / ((rank + 2) as f64).log2())
                            / (2.0f64.powf(relevance) - 1.0)
                    }));
                }
            }
            _ => {}
        }
    }
    let mut warnings = Vec::new();
    let human_count = label_source_counts
        .get(ValidationState::HumanConfirmed.as_str())
        .copied()
        .unwrap_or_default()
        + label_source_counts
            .get(ValidationState::HumanRejected.as_str())
            .copied()
            .unwrap_or_default();
    let confidence = if evaluated.len() < 12 || families.len() < 4 {
        warnings.push("insufficient_local_samples".to_string());
        "low"
    } else if human_count < 5 {
        warnings.push("limited_human_confirmed_labels".to_string());
        "medium"
    } else {
        "high"
    };
    if precision.len() < evaluated.len() {
        warnings.push("metrics_omit_unlabeled_or_conflicted_samples".to_string());
    }
    ValidationMetrics {
        recall_at_k: average_bools(&recall),
        precision_at_k: average(&precision),
        mrr: average(&reciprocal_ranks),
        ndcg: average(&ndcgs),
        no_result_accuracy: average_bools(&no_result),
        forbidden_error_rate: average_bools(&forbidden),
        chunk_evidence_recall: average_bools(&evidence),
        evaluated_samples: evaluated.len() as u64,
        no_result_count: no_result.len() as u64,
        forbidden_count: forbidden.len() as u64,
        evidence_count: evidence.len() as u64,
        confidence: confidence.to_string(),
        warnings,
        advisory_only: true,
        blocks_activation: false,
        label_source_counts,
    }
}

fn average(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn average_bools(values: &[bool]) -> Option<f64> {
    (!values.is_empty())
        .then(|| values.iter().filter(|value| **value).count() as f64 / values.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{CanonicalFile, CanonicalSkill};
    use uuid::Uuid;

    #[test]
    fn family_split_is_stable_and_isolates_paraphrases() {
        let split = split_for_family("skill-family");
        assert_eq!(split, split_for_family("skill-family"));
        let samples = [
            ("first paraphrase", "skill-family"),
            ("second paraphrase", "skill-family"),
        ];
        assert!(samples
            .iter()
            .all(|(_, family)| split_for_family(family) == split));
    }

    #[test]
    fn feedback_rejects_pixels_and_invalid_semantics() {
        assert!(serde_json::from_value::<FeedbackRequest>(json!({
            "action": "move_closer",
            "skillId": "a",
            "otherSkillId": "b",
            "relationType": "similar_to",
            "x": 20,
            "y": 30
        }))
        .is_err());
        let request: FeedbackRequest = serde_json::from_value(json!({
            "action": "move_closer",
            "skillId": "a"
        }))
        .unwrap();
        assert!(request.validate().is_err());
    }

    #[test]
    fn insufficient_data_lowers_confidence_without_inventing_metrics() {
        let metrics = compute_metrics(&[], 5);
        assert_eq!(metrics.confidence, "low");
        assert!(metrics
            .warnings
            .contains(&"insufficient_local_samples".to_string()));
        assert!(metrics.recall_at_k.is_none());
        assert!(!metrics.blocks_activation);
    }

    #[test]
    fn generated_consensus_is_provisional_and_uses_only_user_snapshot() {
        let root = std::env::temp_dir().join(format!("deadalus-validation-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("SKILL.md"),
            "---\nname: local-skill\ndescription: Local only\n---\n# Trigger\nDo not use for images.\n# Workflow\nRead local text.",
        )
        .unwrap();
        let snapshot = CanonicalSnapshot {
            snapshot_id: "user-snapshot".to_string(),
            content_hash: "user-content".to_string(),
            scan_started_at: 1,
            scan_completed_at: 2,
            searched_paths: vec![root.to_string_lossy().into_owned()],
            warnings: vec![],
            skills: vec![CanonicalSkill {
                skill_id: "local-skill".to_string(),
                name: "local-skill".to_string(),
                description: Some("Local only".to_string()),
                path: root.to_string_lossy().into_owned(),
                source_path: root.to_string_lossy().into_owned(),
                scope: "user".to_string(),
                is_built_in: false,
                enabled_agents: vec!["cursor".to_string()],
                in_library: true,
                library_path: None,
                content_hash: "skill-content".to_string(),
                files: vec![CanonicalFile {
                    file_id: "skill-file".to_string(),
                    relative_path: "SKILL.md".to_string(),
                    media_type: Some("text/markdown".to_string()),
                    extension: Some("md".to_string()),
                    content_hash: "file-content".to_string(),
                    size_bytes: 128,
                    is_embeddable: true,
                    modified_at: None,
                    semantic_role: None,
                }],
            }],
        };
        let analyses = vec![SavedAnalysisRecord {
            skill_id: "local-skill".to_string(),
            rule_result: Some(json!({"fields": {"workflow": ["Read local text."]}})),
            comparison_status: Some("consistent".to_string()),
            llm_result: Some(json!({
                "fields": {
                    "workflow": ["Find local text workflows"],
                    "no_result": ["A domain absent from these local Skills"]
                }
            })),
        }];
        let samples = generate_samples(&snapshot, &analyses, Some("profile"), 3);
        assert!(!samples.is_empty());
        assert!(samples
            .iter()
            .all(|sample| sample.dataset_id.starts_with("user-local:user-snapshot")));
        assert!(samples
            .iter()
            .any(|sample| sample.state == ValidationState::MachineConsensus));
        assert!(samples
            .iter()
            .filter(|sample| sample.state == ValidationState::MachineConsensus)
            .all(|sample| sample.confidence.unwrap_or_default() < 1.0));
        assert!(!samples.iter().any(|sample| matches!(
            sample.state,
            ValidationState::HumanConfirmed | ValidationState::HumanRejected
        )));
        assert!(samples.iter().any(|sample| sample.task_type == "no_result"));
        assert!(samples.iter().any(|sample| sample.task_type == "forbidden"));
        assert!(samples
            .iter()
            .all(|sample| sample.split == split_for_family("local-skill")));
        std::fs::remove_dir_all(root).unwrap();
    }
}
