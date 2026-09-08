use crate::application::{ClusterSemantic, SemanticRecordStatus, SkillClassification};
use crate::database::{CanonicalSkill, CanonicalSnapshot};
use crate::vectorization::{
    stable_hash, RelationshipState, RelationshipType, SkillRelationship, StoredVector, VectorLevel,
    VectorStatus, VectorType,
};
use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub const VISUAL_SIMILARITY_THRESHOLD: f32 = 0.72;
pub const VISUAL_EDGE_TOP_K: usize = 5;
pub const CLUSTER_SIMILARITY_THRESHOLD: f32 = 0.78;
pub const CLASSIFIED_VECTOR_FLOOR: f32 = 0.62;
pub const CLASSIFICATION_AFFINITY_THRESHOLD: f32 = 0.20;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillGraphSnapshot {
    pub graph_version: String,
    pub layout_version: String,
    pub profile_id: Option<String>,
    pub view_id: String,
    pub nodes: Vec<SkillGraphNode>,
    pub clusters: Vec<SkillGraphCluster>,
    pub edges: Vec<SkillGraphEdge>,
    pub proximities: Vec<SkillGraphProximity>,
    pub excluded_unready_count: usize,
    pub excluded_unconnected_count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillGraphNode {
    pub skill_id: String,
    pub name: String,
    pub description: Option<String>,
    pub path: String,
    pub enabled_agents: Vec<String>,
    pub cluster_id: Option<String>,
    pub centrality: f32,
    pub superseded: bool,
    pub disabled: bool,
    pub classification_status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillGraphCluster {
    pub cluster_id: String,
    pub name: String,
    pub summary: String,
    pub member_skill_ids: Vec<String>,
    pub core_skill_ids: Vec<String>,
    pub peripheral: bool,
    pub semantic_status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillGraphProximity {
    pub source_skill_id: String,
    pub target_skill_id: String,
    pub weight: f32,
    pub relationship_types: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillGraphEdge {
    pub edge_id: String,
    pub source_skill_id: String,
    pub target_skill_id: String,
    pub similarity: f32,
    pub relations: Vec<SkillGraphRelation>,
    pub nearest_fallback: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillGraphRelation {
    pub relationship_type: String,
    pub vector_type: Option<String>,
    pub state: String,
    pub score: Option<f64>,
    pub source: String,
    pub evidence: Value,
}

#[derive(Debug, Clone)]
struct PairSimilarity {
    source: String,
    target: String,
    score: f32,
}

#[derive(Debug, Clone)]
struct EdgeCandidate {
    source: String,
    target: String,
    similarity: f32,
    relations: Vec<SkillGraphRelation>,
    nearest_fallback: bool,
    has_stored_relation: bool,
}

pub fn empty_graph(view_id: &str) -> SkillGraphSnapshot {
    SkillGraphSnapshot {
        graph_version: format!("no-active-profile:{view_id}"),
        layout_version: format!("no-active-profile:{view_id}"),
        profile_id: None,
        view_id: view_id.to_string(),
        nodes: Vec::new(),
        clusters: Vec::new(),
        edges: Vec::new(),
        proximities: Vec::new(),
        excluded_unready_count: 0,
        excluded_unconnected_count: 0,
    }
}

pub fn build_skill_graph(
    snapshot: &CanonicalSnapshot,
    profile_id: &str,
    vectors: &[StoredVector],
    relationships: &[SkillRelationship],
    view_id: &str,
) -> SkillGraphSnapshot {
    let classifications = snapshot
        .skills
        .iter()
        .map(|skill| synthetic_ready_classification(profile_id, skill))
        .collect::<Vec<_>>();
    build_classified_skill_graph(
        snapshot,
        profile_id,
        vectors,
        relationships,
        &classifications,
        &[],
        view_id,
    )
}

pub fn build_classified_skill_graph(
    snapshot: &CanonicalSnapshot,
    profile_id: &str,
    vectors: &[StoredVector],
    relationships: &[SkillRelationship],
    classifications: &[SkillClassification],
    cluster_semantics: &[ClusterSemantic],
    view_id: &str,
) -> SkillGraphSnapshot {
    let visible_skills = snapshot
        .skills
        .iter()
        .filter(|skill| belongs_to_view(skill, view_id))
        .collect::<Vec<_>>();
    let ready_vectors = ready_overall_vectors(vectors);
    let all_ready_skills = snapshot
        .skills
        .iter()
        .filter_map(|skill| {
            ready_vectors
                .get(&skill.skill_id)
                .map(|vector| (skill.skill_id.clone(), (skill, *vector)))
        })
        .collect::<BTreeMap<_, _>>();
    let ready_skills = visible_skills
        .iter()
        .filter_map(|skill| {
            ready_vectors
                .get(&skill.skill_id)
                .map(|vector| (skill.skill_id.clone(), (*skill, *vector)))
        })
        .collect::<BTreeMap<_, _>>();
    let excluded_unready_count = visible_skills.len().saturating_sub(ready_skills.len());
    let disabled_ids = if view_id == "all" {
        HashSet::new()
    } else {
        ready_skills
            .iter()
            .filter(|(_, (skill, _))| skill.disabled_agents.iter().any(|agent| agent == view_id))
            .map(|(skill_id, _)| skill_id.clone())
            .collect::<HashSet<_>>()
    };
    let classification_by_skill = classifications
        .iter()
        .filter(|classification| classification.profile_id == profile_id)
        .map(|classification| (classification.skill_id.clone(), classification))
        .collect::<HashMap<_, _>>();
    let clusterable_skills = all_ready_skills
        .iter()
        .filter(|(skill_id, _)| {
            classification_by_skill
                .get(*skill_id)
                .is_some_and(|classification| classification.status == SemanticRecordStatus::Ready)
        })
        .map(|(skill_id, value)| (skill_id.clone(), *value))
        .collect::<BTreeMap<_, _>>();
    let relevant_relationships = relationships
        .iter()
        .filter(|relation| relation_belongs_to_profile(relation, profile_id))
        .filter(|relation| relation_is_active(relation))
        .collect::<Vec<_>>();
    let conflict_pairs = relevant_relationships
        .iter()
        .filter(|relation| {
            relation.relationship_type == RelationshipType::ConflictsWith
                || relation.state == RelationshipState::Conflict
        })
        .map(|relation| ordered_pair(&relation.source_skill_id, &relation.target_skill_id))
        .collect::<HashSet<_>>();

    let mut similarities = Vec::new();
    let ready_ids = clusterable_skills.keys().cloned().collect::<Vec<_>>();
    let normalized_vectors = clusterable_skills
        .iter()
        .filter_map(|(skill_id, (_, vector))| {
            normalize(&vector.vector).map(|normalized| (skill_id.clone(), normalized))
        })
        .collect::<HashMap<_, _>>();
    for (index, source_id) in ready_ids.iter().enumerate() {
        for target_id in ready_ids.iter().skip(index + 1) {
            if conflict_pairs.contains(&ordered_pair(source_id, target_id)) {
                continue;
            }
            let (Some(source_vector), Some(target_vector)) = (
                normalized_vectors.get(source_id),
                normalized_vectors.get(target_id),
            ) else {
                continue;
            };
            if source_vector.len() != target_vector.len() {
                continue;
            }
            let score = source_vector
                .iter()
                .zip(target_vector)
                .map(|(left, right)| left * right)
                .sum();
            similarities.push(PairSimilarity {
                source: source_id.clone(),
                target: target_id.clone(),
                score,
            });
        }
    }

    let excluded_unconnected_count = 0;
    let (all_clusters, all_cluster_by_skill, centrality_by_skill) = build_clusters(
        &clusterable_skills,
        &similarities,
        &classification_by_skill,
        cluster_semantics,
    );
    let visible_clusterable_ids = ready_skills
        .keys()
        .filter(|skill_id| {
            !disabled_ids.contains(*skill_id)
                && classification_by_skill
                    .get(*skill_id)
                    .is_some_and(|classification| {
                        classification.status == SemanticRecordStatus::Ready
                    })
        })
        .cloned()
        .collect::<HashSet<_>>();
    let clusters = all_clusters
        .into_iter()
        .filter_map(|mut cluster| {
            cluster
                .member_skill_ids
                .retain(|skill_id| visible_clusterable_ids.contains(skill_id));
            if cluster.member_skill_ids.len() < 2 {
                return None;
            }
            cluster
                .core_skill_ids
                .retain(|skill_id| visible_clusterable_ids.contains(skill_id));
            if cluster.core_skill_ids.is_empty() {
                let mut ranked = cluster.member_skill_ids.clone();
                ranked.sort_by(|left, right| {
                    centrality_by_skill
                        .get(right)
                        .unwrap_or(&0.0)
                        .partial_cmp(centrality_by_skill.get(left).unwrap_or(&0.0))
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| left.cmp(right))
                });
                cluster.core_skill_ids = ranked.into_iter().take(2).collect();
            }
            Some(cluster)
        })
        .collect::<Vec<_>>();
    let retained_clusters = clusters
        .iter()
        .map(|cluster| cluster.cluster_id.clone())
        .collect::<HashSet<_>>();
    let cluster_by_skill = all_cluster_by_skill
        .into_iter()
        .filter(|(skill_id, cluster_id)| {
            visible_clusterable_ids.contains(skill_id) && retained_clusters.contains(cluster_id)
        })
        .collect::<HashMap<_, _>>();
    let superseded_ids = confirmed_superseded_ids(&relevant_relationships);
    let nodes = ready_skills
        .values()
        .map(|(skill, _)| {
            graph_node(
                skill,
                cluster_by_skill.get(&skill.skill_id).cloned(),
                *centrality_by_skill.get(&skill.skill_id).unwrap_or(&0.0),
                superseded_ids.contains(&skill.skill_id),
                disabled_ids.contains(&skill.skill_id),
                classification_by_skill
                    .get(&skill.skill_id)
                    .map(|classification| classification.status.as_str())
                    .unwrap_or("missing"),
            )
        })
        .collect::<Vec<_>>();
    let node_ids = nodes
        .iter()
        .filter(|node| !node.disabled && node.classification_status == "ready")
        .map(|node| node.skill_id.clone())
        .collect::<HashSet<_>>();

    let visible_stored_by_pair = stored_relations_by_pair(
        &relevant_relationships,
        &node_ids,
        &conflict_pairs,
        visible_relationship,
    );
    let detail_relations_by_pair = stored_relations_by_pair(
        &relevant_relationships,
        &node_ids,
        &conflict_pairs,
        |kind| kind != RelationshipType::ConflictsWith,
    );
    let proximities = build_proximities(&relevant_relationships, &node_ids, &conflict_pairs);
    let mut candidates = BTreeMap::<(String, String), EdgeCandidate>::new();
    for pair in similarities
        .iter()
        .filter(|pair| node_ids.contains(&pair.source) && node_ids.contains(&pair.target))
    {
        let key = ordered_pair(&pair.source, &pair.target);
        let visible_stored = visible_stored_by_pair
            .get(&key)
            .cloned()
            .unwrap_or_default();
        if pair.score < VISUAL_SIMILARITY_THRESHOLD && visible_stored.is_empty() {
            continue;
        }
        let mut relations = detail_relations_by_pair
            .get(&key)
            .cloned()
            .unwrap_or_default();
        if pair.score >= VISUAL_SIMILARITY_THRESHOLD {
            relations.push(vector_similarity_relation(pair.score, false));
        }
        candidates.insert(
            key,
            EdgeCandidate {
                source: pair.source.clone(),
                target: pair.target.clone(),
                similarity: pair.score,
                has_stored_relation: !visible_stored.is_empty(),
                relations,
                nearest_fallback: false,
            },
        );
    }

    let selected = select_top_k(candidates.into_values().collect(), &node_ids);
    let edges = selected
        .into_iter()
        .map(|edge| SkillGraphEdge {
            edge_id: graph_edge_id(&edge.source, &edge.target),
            source_skill_id: edge.source,
            target_skill_id: edge.target,
            similarity: edge.similarity,
            relations: edge.relations,
            nearest_fallback: edge.nearest_fallback,
        })
        .collect::<Vec<_>>();
    let graph_version = graph_version(
        profile_id,
        &nodes,
        &clusters,
        &edges,
        &proximities,
        &ready_skills,
    );
    let layout_version = layout_version(profile_id, &nodes, &ready_skills);

    SkillGraphSnapshot {
        graph_version,
        layout_version,
        profile_id: Some(profile_id.to_string()),
        view_id: view_id.to_string(),
        nodes,
        clusters,
        edges,
        proximities,
        excluded_unready_count,
        excluded_unconnected_count,
    }
}

fn synthetic_ready_classification(profile_id: &str, skill: &CanonicalSkill) -> SkillClassification {
    SkillClassification {
        profile_id: profile_id.to_string(),
        skill_id: skill.skill_id.clone(),
        input_hash: skill.content_hash.clone(),
        schema_version: "legacy-test".to_string(),
        provider: "deterministic".to_string(),
        model: "deterministic".to_string(),
        prompt_version: "legacy-test".to_string(),
        broad_category: "legacy-test".to_string(),
        small_categories: vec!["legacy-test".to_string()],
        target_object: "legacy-test".to_string(),
        user_goal: "legacy-test".to_string(),
        capability_summary: skill
            .description
            .clone()
            .unwrap_or_else(|| skill.name.clone()),
        workflow_summary: skill
            .description
            .clone()
            .unwrap_or_else(|| skill.name.clone()),
        confidence: 1.0,
        evidence: Vec::new(),
        status: SemanticRecordStatus::Ready,
        error: None,
        created_at: 0,
        updated_at: 0,
    }
}

fn belongs_to_view(skill: &CanonicalSkill, view_id: &str) -> bool {
    view_id == "all" || skill.enabled_agents.iter().any(|agent| agent == view_id)
}

fn ready_overall_vectors(vectors: &[StoredVector]) -> HashMap<String, &StoredVector> {
    let mut selected = HashMap::new();
    for vector in vectors.iter().filter(|vector| {
        vector.vector_type == VectorType::OverallFunction
            && vector.level == VectorLevel::Parent
            && vector.status == VectorStatus::Ready
    }) {
        selected
            .entry(vector.skill_id.clone())
            .and_modify(|current: &mut &StoredVector| {
                if vector.embedding_id > current.embedding_id {
                    *current = vector;
                }
            })
            .or_insert(vector);
    }
    selected
}

fn graph_node(
    skill: &CanonicalSkill,
    cluster_id: Option<String>,
    centrality: f32,
    superseded: bool,
    disabled: bool,
    classification_status: &str,
) -> SkillGraphNode {
    SkillGraphNode {
        skill_id: skill.skill_id.clone(),
        name: skill.name.clone(),
        description: skill.description.clone(),
        path: skill.path.clone(),
        enabled_agents: skill.enabled_agents.clone(),
        cluster_id,
        centrality,
        superseded,
        disabled,
        classification_status: classification_status.to_string(),
    }
}

fn relation_belongs_to_profile(relation: &SkillRelationship, profile_id: &str) -> bool {
    relation
        .source_profile_id
        .as_deref()
        .is_none_or(|id| id == profile_id)
        && relation
            .target_profile_id
            .as_deref()
            .is_none_or(|id| id == profile_id)
}

fn relation_is_active(relation: &SkillRelationship) -> bool {
    !matches!(
        relation.state,
        RelationshipState::HumanRejected
            | RelationshipState::HumanReverted
            | RelationshipState::Rejected
    )
}

fn stored_relations_by_pair(
    relationships: &[&SkillRelationship],
    node_ids: &HashSet<String>,
    conflict_pairs: &HashSet<(String, String)>,
    include: fn(RelationshipType) -> bool,
) -> BTreeMap<(String, String), Vec<SkillGraphRelation>> {
    let mut grouped = BTreeMap::<_, Vec<_>>::new();
    for relation in relationships {
        let key = ordered_pair(&relation.source_skill_id, &relation.target_skill_id);
        if !include(relation.relationship_type)
            || conflict_pairs.contains(&key)
            || !node_ids.contains(&relation.source_skill_id)
            || !node_ids.contains(&relation.target_skill_id)
        {
            continue;
        }
        grouped.entry(key).or_default().push(SkillGraphRelation {
            relationship_type: relation.relationship_type.as_str().to_string(),
            vector_type: relation.vector_type.map(|kind| kind.as_str().to_string()),
            state: relation.state.as_str().to_string(),
            score: relation.score,
            source: "stored".to_string(),
            evidence: relation.evidence.clone(),
        });
    }
    grouped
}

fn visible_relationship(kind: RelationshipType) -> bool {
    matches!(
        kind,
        RelationshipType::SimilarTo | RelationshipType::DependsOn | RelationshipType::LocatedIn
    )
}

fn weak_relationship(kind: RelationshipType) -> bool {
    matches!(
        kind,
        RelationshipType::OverlapsWith
            | RelationshipType::ReadsReference
            | RelationshipType::RunsScript
            | RelationshipType::UsesAsset
            | RelationshipType::Supersedes
            | RelationshipType::DuplicateCandidate
    )
}

fn confirmed_superseded_ids(relationships: &[&SkillRelationship]) -> HashSet<String> {
    relationships
        .iter()
        .filter(|relation| relation.relationship_type == RelationshipType::Supersedes)
        .filter(|relation| {
            matches!(
                relation.state,
                RelationshipState::HumanConfirmed
                    | RelationshipState::Revalidated
                    | RelationshipState::CarriedFact
            )
        })
        .map(|relation| relation.target_skill_id.clone())
        .collect()
}

fn build_proximities(
    relationships: &[&SkillRelationship],
    node_ids: &HashSet<String>,
    conflict_pairs: &HashSet<(String, String)>,
) -> Vec<SkillGraphProximity> {
    let mut grouped = BTreeMap::<(String, String), (f32, BTreeSet<String>)>::new();
    for relation in relationships.iter().filter(|relation| {
        weak_relationship(relation.relationship_type)
            && node_ids.contains(&relation.source_skill_id)
            && node_ids.contains(&relation.target_skill_id)
    }) {
        let key = ordered_pair(&relation.source_skill_id, &relation.target_skill_id);
        if conflict_pairs.contains(&key) {
            continue;
        }
        let default_weight = match relation.relationship_type {
            RelationshipType::OverlapsWith => 0.55,
            RelationshipType::Supersedes | RelationshipType::DuplicateCandidate => 0.4,
            _ => 0.22,
        };
        let entry = grouped.entry(key).or_default();
        entry.0 = (entry.0 + relation.score.unwrap_or(default_weight) as f32).min(1.0);
        entry
            .1
            .insert(relation.relationship_type.as_str().to_string());
    }
    grouped
        .into_iter()
        .map(
            |((source_skill_id, target_skill_id), (weight, kinds))| SkillGraphProximity {
                source_skill_id,
                target_skill_id,
                weight,
                relationship_types: kinds.into_iter().collect(),
            },
        )
        .collect()
}

fn build_clusters(
    ready_skills: &BTreeMap<String, (&CanonicalSkill, &StoredVector)>,
    similarities: &[PairSimilarity],
    classifications: &HashMap<String, &SkillClassification>,
    cluster_semantics: &[ClusterSemantic],
) -> (
    Vec<SkillGraphCluster>,
    HashMap<String, String>,
    HashMap<String, f32>,
) {
    let mut adjacency = ready_skills
        .keys()
        .map(|id| (id.clone(), Vec::<String>::new()))
        .collect::<HashMap<_, _>>();
    for pair in similarities.iter().filter(|pair| {
        let (Some(source), Some(target)) = (
            classifications.get(&pair.source),
            classifications.get(&pair.target),
        ) else {
            return false;
        };
        category_compatible(source, target, pair.score)
    }) {
        adjacency
            .entry(pair.source.clone())
            .or_default()
            .push(pair.target.clone());
        adjacency
            .entry(pair.target.clone())
            .or_default()
            .push(pair.source.clone());
    }

    let mut visited = HashSet::new();
    let mut components = Vec::<Vec<String>>::new();
    for skill_id in ready_skills.keys() {
        if !visited.insert(skill_id.clone()) {
            continue;
        }
        let mut stack = vec![skill_id.clone()];
        let mut members = Vec::new();
        while let Some(current) = stack.pop() {
            members.push(current.clone());
            for neighbor in adjacency.get(&current).into_iter().flatten() {
                if visited.insert(neighbor.clone()) {
                    stack.push(neighbor.clone());
                }
            }
        }
        members.sort();
        components.push(members);
    }

    let mut cluster_by_skill = HashMap::new();
    let mut centrality_by_skill = HashMap::new();
    let mut clusters = Vec::new();
    for members in components.into_iter().filter(|members| members.len() >= 2) {
        let cluster_id = format!(
            "cluster_{}",
            &stable_hash(members.join("\0").as_bytes())[..20]
        );
        let member_set = members.iter().cloned().collect::<HashSet<_>>();
        for member in &members {
            let scores = similarities
                .iter()
                .filter(|pair| {
                    (pair.source == *member && member_set.contains(&pair.target))
                        || (pair.target == *member && member_set.contains(&pair.source))
                })
                .map(|pair| pair.score)
                .collect::<Vec<_>>();
            let centrality = if scores.is_empty() {
                0.0
            } else {
                scores.iter().sum::<f32>() / scores.len() as f32
            };
            centrality_by_skill.insert(member.clone(), centrality);
            cluster_by_skill.insert(member.clone(), cluster_id.clone());
        }
        let core_count = ((members.len() as f32).sqrt().ceil() as usize).clamp(2, 4);
        let mut ranked = members.clone();
        ranked.sort_by(|left, right| {
            centrality_by_skill[right]
                .partial_cmp(&centrality_by_skill[left])
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.cmp(right))
        });
        let member_classifications = members
            .iter()
            .filter_map(|id| classifications.get(id).copied())
            .collect::<Vec<_>>();
        let temporary_name = temporary_cluster_name(&member_classifications);
        let semantic = cluster_semantics
            .iter()
            .find(|semantic| semantic.cluster_id == cluster_id);
        let (name, summary, semantic_status) = match semantic {
            Some(semantic) if semantic.status == SemanticRecordStatus::Ready => (
                semantic.name.clone(),
                semantic.summary.clone(),
                semantic.status.as_str().to_string(),
            ),
            Some(semantic) => (
                temporary_name,
                "集群语义已过期，等待重新生成。".to_string(),
                semantic.status.as_str().to_string(),
            ),
            None => (
                temporary_name,
                "临时分类名称，等待 LLM 生成集群概述。".to_string(),
                "pending".to_string(),
            ),
        };
        clusters.push(SkillGraphCluster {
            cluster_id,
            name,
            summary,
            member_skill_ids: members,
            core_skill_ids: ranked.into_iter().take(core_count).collect(),
            peripheral: false,
            semantic_status,
        });
    }
    clusters.sort_by(|left, right| left.cluster_id.cmp(&right.cluster_id));
    (clusters, cluster_by_skill, centrality_by_skill)
}

fn category_compatible(
    source: &SkillClassification,
    target: &SkillClassification,
    vector_similarity: f32,
) -> bool {
    let source_broad = normalize_label(&source.broad_category);
    let target_broad = normalize_label(&target.broad_category);
    let broad_affinity = jaccard(
        &semantic_terms(&source.broad_category),
        &semantic_terms(&target.broad_category),
    );
    // Broad categories are a strong boundary, but not a brittle enum: the LLM
    // may produce near-synonyms such as “网页前端” and “前端界面开发”.  Exact
    // labels always match; otherwise substantial semantic overlap is required.
    // Generic shared words such as “设计” are insufficient on their own.
    if source_broad != target_broad && broad_affinity < 0.45 {
        return false;
    }
    let required_affinity = if vector_similarity >= CLUSTER_SIMILARITY_THRESHOLD {
        CLASSIFICATION_AFFINITY_THRESHOLD * 0.75
    } else {
        CLASSIFICATION_AFFINITY_THRESHOLD
    };
    classification_affinity(source, target) >= required_affinity
        && vector_similarity >= CLASSIFIED_VECTOR_FLOOR
}

fn classification_affinity(source: &SkillClassification, target: &SkillClassification) -> f32 {
    let source_small = source
        .small_categories
        .iter()
        .flat_map(|value| semantic_terms(value))
        .collect::<BTreeSet<_>>();
    let target_small = target
        .small_categories
        .iter()
        .flat_map(|value| semantic_terms(value))
        .collect::<BTreeSet<_>>();
    0.45 * jaccard(&source_small, &target_small)
        + 0.25
            * jaccard(
                &semantic_terms(&source.target_object),
                &semantic_terms(&target.target_object),
            )
        + 0.15
            * jaccard(
                &semantic_terms(&source.user_goal),
                &semantic_terms(&target.user_goal),
            )
        + 0.15
            * jaccard(
                &semantic_terms(&source.workflow_summary),
                &semantic_terms(&target.workflow_summary),
            )
}

fn normalize_label(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn semantic_terms(value: &str) -> BTreeSet<String> {
    let mut terms = value
        .split(|character: char| !character.is_alphanumeric())
        .map(normalize_label)
        .filter(|term| !term.is_empty())
        .collect::<BTreeSet<_>>();
    let compact = normalize_label(value);
    if !compact.is_empty() {
        terms.insert(compact.clone());
    }
    let characters = compact.chars().collect::<Vec<_>>();
    if characters.iter().any(|character| !character.is_ascii()) {
        for pair in characters.windows(2) {
            terms.insert(pair.iter().collect());
        }
    }
    terms
}

fn jaccard(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    left.intersection(right).count() as f32 / left.union(right).count() as f32
}

fn temporary_cluster_name(classifications: &[&SkillClassification]) -> String {
    let mut counts = BTreeMap::<String, (String, usize)>::new();
    for classification in classifications {
        let key = normalize_label(&classification.broad_category);
        let entry = counts
            .entry(key)
            .or_insert_with(|| (classification.broad_category.clone(), 0));
        entry.1 += 1;
    }
    counts
        .into_values()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
        .map(|(name, _)| format!("{name} · 待生成"))
        .unwrap_or_else(|| "功能集群 · 待生成".to_string())
}

pub fn cluster_member_hash(
    member_skill_ids: &[String],
    classifications: &[SkillClassification],
) -> String {
    let by_skill = classifications
        .iter()
        .map(|item| (item.skill_id.as_str(), item))
        .collect::<HashMap<_, _>>();
    let mut members = member_skill_ids.to_vec();
    members.sort();
    let values = members
        .iter()
        .map(|skill_id| {
            by_skill.get(skill_id.as_str()).map_or_else(
                || skill_id.clone(),
                |item| format!("{}:{}", item.skill_id, item.input_hash),
            )
        })
        .collect::<Vec<_>>();
    stable_hash(values.join("\0").as_bytes())
}

fn vector_similarity_relation(score: f32, fallback: bool) -> SkillGraphRelation {
    SkillGraphRelation {
        relationship_type: "similar_to".to_string(),
        vector_type: Some(VectorType::OverallFunction.as_str().to_string()),
        state: if fallback {
            "nearest_fallback".to_string()
        } else {
            "over_threshold".to_string()
        },
        score: Some(f64::from(score)),
        source: "vector_similarity".to_string(),
        evidence: serde_json::json!({
            "threshold": VISUAL_SIMILARITY_THRESHOLD,
            "nearestFallback": fallback,
        }),
    }
}

fn select_top_k(
    mut candidates: Vec<EdgeCandidate>,
    node_ids: &HashSet<String>,
) -> Vec<EdgeCandidate> {
    candidates.sort_by(|left, right| {
        right
            .has_stored_relation
            .cmp(&left.has_stored_relation)
            .then_with(|| {
                right
                    .similarity
                    .partial_cmp(&left.similarity)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| left.target.cmp(&right.target))
    });
    let mut selected = Vec::new();
    let mut degree = node_ids
        .iter()
        .map(|id| (id.clone(), 0usize))
        .collect::<HashMap<_, _>>();
    for candidate in &candidates {
        if degree[&candidate.source] >= VISUAL_EDGE_TOP_K
            || degree[&candidate.target] >= VISUAL_EDGE_TOP_K
        {
            continue;
        }
        selected.push(candidate.clone());
        *degree.get_mut(&candidate.source).expect("source degree") += 1;
        *degree.get_mut(&candidate.target).expect("target degree") += 1;
    }
    for skill_id in node_ids {
        let has_functional_similarity = selected.iter().any(|edge| {
            (edge.source == *skill_id || edge.target == *skill_id)
                && edge
                    .relations
                    .iter()
                    .any(|relation| relation.source == "vector_similarity")
        });
        if has_functional_similarity {
            continue;
        }
        if let Some(candidate) = candidates
            .iter()
            .filter(|edge| {
                (edge.source == *skill_id || edge.target == *skill_id)
                    && edge
                        .relations
                        .iter()
                        .any(|relation| relation.source == "vector_similarity")
            })
            .max_by(|left, right| {
                left.similarity
                    .partial_cmp(&right.similarity)
                    .unwrap_or(Ordering::Equal)
            })
        {
            let key = ordered_pair(&candidate.source, &candidate.target);
            if !selected
                .iter()
                .any(|edge| ordered_pair(&edge.source, &edge.target) == key)
            {
                selected.push(candidate.clone());
                *degree.get_mut(&candidate.source).expect("source degree") += 1;
                *degree.get_mut(&candidate.target).expect("target degree") += 1;
            }
        }
    }
    selected.sort_by(|left, right| {
        left.source
            .cmp(&right.source)
            .then_with(|| left.target.cmp(&right.target))
    });
    selected
}

fn graph_version(
    profile_id: &str,
    nodes: &[SkillGraphNode],
    clusters: &[SkillGraphCluster],
    edges: &[SkillGraphEdge],
    proximities: &[SkillGraphProximity],
    ready_skills: &BTreeMap<String, (&CanonicalSkill, &StoredVector)>,
) -> String {
    let mut parts = vec![profile_id.to_string()];
    for node in nodes {
        let vector_version = ready_skills
            .get(&node.skill_id)
            .map(|(_, vector)| format!("{}:{}", vector.embedding_id, vector.input_hash))
            .unwrap_or_default();
        parts.push(format!(
            "{}:{}:{}:{}:{}:{}:{}:{}",
            node.skill_id,
            node.name,
            node.description.as_deref().unwrap_or_default(),
            node.path,
            node.enabled_agents.join(","),
            node.disabled,
            node.classification_status,
            vector_version,
        ));
    }
    for cluster in clusters {
        parts.push(format!(
            "{}:{}:{}:{}:{}:{}",
            cluster.cluster_id,
            cluster.name,
            cluster.summary,
            cluster.member_skill_ids.join(","),
            cluster.core_skill_ids.join(","),
            cluster.semantic_status,
        ));
    }
    for edge in edges {
        parts.push(format!(
            "{}:{}:{}:{}:{}",
            edge.edge_id,
            edge.source_skill_id,
            edge.target_skill_id,
            edge.similarity.to_bits(),
            edge.nearest_fallback,
        ));
        for relation in &edge.relations {
            parts.push(format!(
                "{}:{}:{}:{:?}:{}",
                relation.relationship_type,
                relation.vector_type.as_deref().unwrap_or_default(),
                relation.state,
                relation.score,
                relation.evidence,
            ));
        }
    }
    for proximity in proximities {
        parts.push(format!(
            "{}:{}:{}:{}",
            proximity.source_skill_id,
            proximity.target_skill_id,
            proximity.weight.to_bits(),
            proximity.relationship_types.join(","),
        ));
    }
    stable_hash(parts.join("\n").as_bytes())
}

fn layout_version(
    profile_id: &str,
    nodes: &[SkillGraphNode],
    ready_skills: &BTreeMap<String, (&CanonicalSkill, &StoredVector)>,
) -> String {
    let mut parts = vec![profile_id.to_string()];
    for node in nodes {
        let vector_version = ready_skills
            .get(&node.skill_id)
            .map(|(_, vector)| format!("{}:{}", vector.embedding_id, vector.input_hash))
            .unwrap_or_default();
        parts.push(format!(
            "{}:{}:{}:{}",
            node.skill_id, node.disabled, node.classification_status, vector_version
        ));
    }
    stable_hash(parts.join("\n").as_bytes())
}

fn ordered_pair(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_string(), right.to_string())
    } else {
        (right.to_string(), left.to_string())
    }
}

fn graph_edge_id(source: &str, target: &str) -> String {
    let (source, target) = ordered_pair(source, target);
    format!(
        "edge_{}",
        &stable_hash(format!("{source}\0{target}").as_bytes())[..24]
    )
}

fn normalize(vector: &[f32]) -> Option<Vec<f32>> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return None;
    }
    Some(vector.iter().map(|value| value / norm).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::CanonicalFile;
    use crate::vectorization::{VectorStatus, VectorType};

    fn skill(id: &str, agent: &str) -> CanonicalSkill {
        CanonicalSkill {
            skill_id: id.to_string(),
            name: id.to_string(),
            description: Some(format!("{id} description")),
            path: format!("C:/{id}"),
            source_path: "test".to_string(),
            scope: "user".to_string(),
            is_built_in: false,
            enabled_agents: vec![agent.to_string()],
            disabled_agents: Vec::new(),
            in_library: false,
            library_path: None,
            backup_suppressed: false,
            content_hash: format!("hash-{id}"),
            files: Vec::<CanonicalFile>::new(),
        }
    }

    fn vector(id: &str, values: Vec<f32>) -> StoredVector {
        StoredVector {
            embedding_id: format!("embedding-{id}"),
            profile_id: "profile".to_string(),
            skill_id: id.to_string(),
            vector_type: VectorType::OverallFunction,
            level: VectorLevel::Parent,
            parent_embedding_id: None,
            resource_category: None,
            chunk_id: None,
            input_hash: format!("input-{id}"),
            vector: values,
            status: VectorStatus::Ready,
            heading_path: None,
            source_file: None,
        }
    }

    fn classification(id: &str, broad: &str, small: &[&str], target: &str) -> SkillClassification {
        SkillClassification {
            profile_id: "profile".to_string(),
            skill_id: id.to_string(),
            input_hash: format!("classification-{id}"),
            schema_version: "v1".to_string(),
            provider: "test".to_string(),
            model: "test".to_string(),
            prompt_version: "v1".to_string(),
            broad_category: broad.to_string(),
            small_categories: small.iter().map(|value| (*value).to_string()).collect(),
            target_object: target.to_string(),
            user_goal: format!("完成 {target}"),
            capability_summary: format!("处理 {target}"),
            workflow_summary: small.join(" "),
            confidence: 0.9,
            evidence: Vec::new(),
            status: SemanticRecordStatus::Ready,
            error: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn snapshot(skills: Vec<CanonicalSkill>) -> CanonicalSnapshot {
        CanonicalSnapshot {
            snapshot_id: "snapshot".to_string(),
            content_hash: "hash".to_string(),
            scan_started_at: 1,
            scan_completed_at: 2,
            searched_paths: Vec::new(),
            warnings: Vec::new(),
            skills,
        }
    }

    fn conflict(left: &str, right: &str) -> SkillRelationship {
        SkillRelationship {
            relation_id: format!("conflict-{left}-{right}"),
            source_skill_id: left.to_string(),
            target_skill_id: right.to_string(),
            relationship_type: RelationshipType::ConflictsWith,
            vector_type: None,
            source_profile_id: Some("profile".to_string()),
            target_profile_id: Some("profile".to_string()),
            score: None,
            state: RelationshipState::Conflict,
            evidence: Value::Null,
            created_at: 1,
            validated_at: None,
        }
    }

    fn relation(
        left: &str,
        right: &str,
        relationship_type: RelationshipType,
        state: RelationshipState,
    ) -> SkillRelationship {
        SkillRelationship {
            relation_id: format!("relation-{left}-{right}-{}", relationship_type.as_str()),
            source_skill_id: left.to_string(),
            target_skill_id: right.to_string(),
            relationship_type,
            vector_type: None,
            source_profile_id: Some("profile".to_string()),
            target_profile_id: Some("profile".to_string()),
            score: Some(0.8),
            state,
            evidence: Value::Null,
            created_at: 1,
            validated_at: None,
        }
    }

    #[test]
    fn only_over_threshold_non_conflicting_pairs_get_similarity_edges() {
        let graph = build_skill_graph(
            &snapshot(vec![
                skill("a", "cursor"),
                skill("b", "cursor"),
                skill("c", "cursor"),
            ]),
            "profile",
            &[
                vector("a", vec![1.0, 0.0]),
                vector("b", vec![0.9, 0.1]),
                vector("c", vec![-1.0, 0.0]),
            ],
            &[],
            "cursor",
        );
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 1);
        assert!(graph.edges[0]
            .relations
            .iter()
            .any(|relation| relation.source == "vector_similarity"));
        assert!(!graph.edges[0].nearest_fallback);
        assert_eq!(graph.clusters.len(), 1);
        assert!(graph
            .nodes
            .iter()
            .find(|node| node.skill_id == "c")
            .unwrap()
            .cluster_id
            .is_none());
    }

    #[test]
    fn classification_prevents_generic_design_similarity_from_merging_domains() {
        let source = snapshot(vec![
            skill("web-design", "cursor"),
            skill("web-a11y", "cursor"),
            skill("game-design", "cursor"),
        ]);
        let vectors = [
            vector("web-design", vec![1.0, 0.0]),
            vector("web-a11y", vec![0.99, 0.01]),
            vector("game-design", vec![0.995, 0.005]),
        ];
        let classifications = [
            classification(
                "web-design",
                "网页前端设计",
                &["设计系统", "界面实现"],
                "网页界面",
            ),
            classification(
                "web-a11y",
                "网页前端设计",
                &["界面实现", "无障碍"],
                "网页界面",
            ),
            classification(
                "game-design",
                "多人游戏设计",
                &["玩法系统", "网络协同"],
                "多人游戏",
            ),
        ];
        let graph = build_classified_skill_graph(
            &source,
            "profile",
            &vectors,
            &[],
            &classifications,
            &[],
            "all",
        );
        assert_eq!(graph.clusters.len(), 1);
        assert_eq!(graph.clusters[0].member_skill_ids.len(), 2);
        assert!(graph.clusters[0]
            .member_skill_ids
            .contains(&"web-design".to_string()));
        assert!(graph.clusters[0]
            .member_skill_ids
            .contains(&"web-a11y".to_string()));
        assert!(graph
            .nodes
            .iter()
            .find(|node| node.skill_id == "game-design")
            .is_some_and(|node| node.cluster_id.is_none()));
    }

    #[test]
    fn conflict_suppresses_all_edges_and_unconnectable_nodes() {
        let graph = build_skill_graph(
            &snapshot(vec![skill("a", "cursor"), skill("b", "cursor")]),
            "profile",
            &[vector("a", vec![1.0, 0.0]), vector("b", vec![0.9, 0.1])],
            &[conflict("a", "b")],
            "cursor",
        );
        assert_eq!(graph.nodes.len(), 2);
        assert!(graph.edges.is_empty());
        assert_eq!(graph.excluded_unconnected_count, 0);
        assert!(graph.clusters.is_empty());
    }

    #[test]
    fn graph_filters_by_agent_and_requires_ready_overall_parent() {
        let mut expired = vector("b", vec![0.9, 0.1]);
        expired.status = VectorStatus::Expired;
        let graph = build_skill_graph(
            &snapshot(vec![
                skill("a", "cursor"),
                skill("b", "cursor"),
                skill("c", "claude-code"),
            ]),
            "profile",
            &[
                vector("a", vec![1.0, 0.0]),
                expired,
                vector("c", vec![1.0, 0.0]),
            ],
            &[],
            "cursor",
        );
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].skill_id, "a");
        assert_eq!(graph.excluded_unready_count, 1);
        assert_eq!(graph.excluded_unconnected_count, 0);
    }

    #[test]
    fn a_single_ready_agent_skill_remains_visible_without_an_edge() {
        let graph = build_skill_graph(
            &snapshot(vec![
                skill("claude", "claude-code"),
                skill("codex", "codex"),
            ]),
            "profile",
            &[
                vector("claude", vec![1.0, 0.0]),
                vector("codex", vec![0.0, 1.0]),
            ],
            &[],
            "claude-code",
        );
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].skill_id, "claude");
        assert!(graph.edges.is_empty());
        assert_eq!(graph.excluded_unconnected_count, 0);
        assert!(graph.nodes[0].cluster_id.is_none());
    }

    #[test]
    fn disabled_skill_remains_visible_only_in_its_agent_view_without_relations() {
        let mut disabled = skill("disabled", "cursor");
        disabled.disabled_agents.push("cursor".to_string());
        let source = snapshot(vec![disabled, skill("active", "cursor")]);
        let vectors = [
            vector("disabled", vec![1.0, 0.0]),
            vector("active", vec![0.99, 0.01]),
        ];
        let agent_graph = build_skill_graph(&source, "profile", &vectors, &[], "cursor");
        assert_eq!(agent_graph.nodes.len(), 2);
        assert!(
            agent_graph
                .nodes
                .iter()
                .find(|node| node.skill_id == "disabled")
                .unwrap()
                .disabled
        );
        assert!(agent_graph.edges.is_empty());
        assert!(agent_graph.proximities.is_empty());

        let all_graph = build_skill_graph(&source, "profile", &vectors, &[], "all");
        assert!(!all_graph.nodes.iter().any(|node| node.disabled));
        assert_eq!(all_graph.edges.len(), 1);
    }

    #[test]
    fn weak_relations_move_nodes_without_creating_an_edge_and_remain_available_as_detail() {
        let weak = relation(
            "a",
            "b",
            RelationshipType::OverlapsWith,
            RelationshipState::Revalidated,
        );
        let below_threshold = build_skill_graph(
            &snapshot(vec![skill("a", "cursor"), skill("b", "cursor")]),
            "profile",
            &[vector("a", vec![1.0, 0.0]), vector("b", vec![0.0, 1.0])],
            std::slice::from_ref(&weak),
            "cursor",
        );
        assert!(below_threshold.edges.is_empty());
        assert_eq!(below_threshold.proximities.len(), 1);

        let over_threshold = build_skill_graph(
            &snapshot(vec![skill("a", "cursor"), skill("b", "cursor")]),
            "profile",
            &[vector("a", vec![1.0, 0.0]), vector("b", vec![0.9, 0.1])],
            &[weak],
            "cursor",
        );
        assert_eq!(over_threshold.edges.len(), 1);
        assert!(over_threshold.edges[0]
            .relations
            .iter()
            .any(|item| item.relationship_type == "overlaps_with"));
    }

    #[test]
    fn confirmed_supersedes_dims_only_the_replaced_target() {
        let graph = build_skill_graph(
            &snapshot(vec![skill("new", "cursor"), skill("old", "cursor")]),
            "profile",
            &[vector("new", vec![1.0, 0.0]), vector("old", vec![0.9, 0.1])],
            &[relation(
                "new",
                "old",
                RelationshipType::Supersedes,
                RelationshipState::HumanConfirmed,
            )],
            "cursor",
        );
        assert!(
            !graph
                .nodes
                .iter()
                .find(|node| node.skill_id == "new")
                .unwrap()
                .superseded
        );
        assert!(
            graph
                .nodes
                .iter()
                .find(|node| node.skill_id == "old")
                .unwrap()
                .superseded
        );
    }
}
