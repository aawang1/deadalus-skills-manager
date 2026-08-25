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

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillGraphSnapshot {
    pub graph_version: String,
    pub profile_id: Option<String>,
    pub view_id: String,
    pub nodes: Vec<SkillGraphNode>,
    pub edges: Vec<SkillGraphEdge>,
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
        profile_id: None,
        view_id: view_id.to_string(),
        nodes: Vec::new(),
        edges: Vec::new(),
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
    let visible_skills = snapshot
        .skills
        .iter()
        .filter(|skill| belongs_to_view(skill, view_id))
        .collect::<Vec<_>>();
    let ready_vectors = ready_overall_vectors(vectors);
    let ready_skills = visible_skills
        .iter()
        .filter_map(|skill| {
            ready_vectors
                .get(&skill.skill_id)
                .map(|vector| (skill.skill_id.clone(), (*skill, *vector)))
        })
        .collect::<BTreeMap<_, _>>();
    let excluded_unready_count = visible_skills.len().saturating_sub(ready_skills.len());
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
    let ready_ids = ready_skills.keys().cloned().collect::<Vec<_>>();
    let normalized_vectors = ready_skills
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

    let connected_ids = similarities
        .iter()
        .flat_map(|pair| [&pair.source, &pair.target])
        .cloned()
        .collect::<BTreeSet<_>>();
    let excluded_unconnected_count = ready_skills.len().saturating_sub(connected_ids.len());
    let nodes = ready_skills
        .values()
        .map(|(skill, _)| graph_node(skill))
        .collect::<Vec<_>>();
    let node_ids = nodes
        .iter()
        .map(|node| node.skill_id.clone())
        .collect::<HashSet<_>>();

    let stored_by_pair =
        stored_relations_by_pair(&relevant_relationships, &node_ids, &conflict_pairs);
    let nearest_by_node = nearest_pairs(&similarities, &node_ids);
    let mut candidates = BTreeMap::<(String, String), EdgeCandidate>::new();
    for pair in similarities
        .iter()
        .filter(|pair| node_ids.contains(&pair.source) && node_ids.contains(&pair.target))
    {
        let key = ordered_pair(&pair.source, &pair.target);
        let stored = stored_by_pair.get(&key).cloned().unwrap_or_default();
        let nearest_fallback = nearest_by_node.get(&pair.source) == Some(&key)
            || nearest_by_node.get(&pair.target) == Some(&key);
        if pair.score < VISUAL_SIMILARITY_THRESHOLD && stored.is_empty() && !nearest_fallback {
            continue;
        }
        let mut relations = stored;
        if pair.score >= VISUAL_SIMILARITY_THRESHOLD || nearest_fallback {
            relations.push(vector_similarity_relation(pair.score, nearest_fallback));
        }
        candidates.insert(
            key,
            EdgeCandidate {
                source: pair.source.clone(),
                target: pair.target.clone(),
                similarity: pair.score,
                has_stored_relation: !relations
                    .iter()
                    .all(|relation| relation.source == "vector_similarity"),
                relations,
                nearest_fallback: pair.score < VISUAL_SIMILARITY_THRESHOLD && nearest_fallback,
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
    let graph_version = graph_version(profile_id, &nodes, &edges, &ready_skills);

    SkillGraphSnapshot {
        graph_version,
        profile_id: Some(profile_id.to_string()),
        view_id: view_id.to_string(),
        nodes,
        edges,
        excluded_unready_count,
        excluded_unconnected_count,
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

fn graph_node(skill: &CanonicalSkill) -> SkillGraphNode {
    SkillGraphNode {
        skill_id: skill.skill_id.clone(),
        name: skill.name.clone(),
        description: skill.description.clone(),
        path: skill.path.clone(),
        enabled_agents: skill.enabled_agents.clone(),
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
) -> BTreeMap<(String, String), Vec<SkillGraphRelation>> {
    let mut grouped = BTreeMap::<_, Vec<_>>::new();
    for relation in relationships {
        let key = ordered_pair(&relation.source_skill_id, &relation.target_skill_id);
        if conflict_pairs.contains(&key)
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

fn nearest_pairs(
    similarities: &[PairSimilarity],
    node_ids: &HashSet<String>,
) -> HashMap<String, (String, String)> {
    let mut nearest = HashMap::<String, (&PairSimilarity, f32)>::new();
    for pair in similarities
        .iter()
        .filter(|pair| node_ids.contains(&pair.source) && node_ids.contains(&pair.target))
    {
        for skill_id in [&pair.source, &pair.target] {
            let replace = nearest
                .get(skill_id)
                .is_none_or(|(_, score)| pair.score > *score);
            if replace {
                nearest.insert(skill_id.clone(), (pair, pair.score));
            }
        }
    }
    nearest
        .into_iter()
        .map(|(skill_id, (pair, _))| (skill_id, ordered_pair(&pair.source, &pair.target)))
        .collect()
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
    edges: &[SkillGraphEdge],
    ready_skills: &BTreeMap<String, (&CanonicalSkill, &StoredVector)>,
) -> String {
    let mut parts = vec![profile_id.to_string()];
    for node in nodes {
        let vector_version = ready_skills
            .get(&node.skill_id)
            .map(|(_, vector)| format!("{}:{}", vector.embedding_id, vector.input_hash))
            .unwrap_or_default();
        parts.push(format!(
            "{}:{}:{}:{}:{}:{}",
            node.skill_id,
            node.name,
            node.description.as_deref().unwrap_or_default(),
            node.path,
            node.enabled_agents.join(","),
            vector_version,
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
            in_library: false,
            library_path: None,
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

    #[test]
    fn every_visible_node_gets_a_non_conflicting_similarity_edge() {
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
        assert!(graph
            .nodes
            .iter()
            .all(|node| graph.edges.iter().any(|edge| {
                (edge.source_skill_id == node.skill_id || edge.target_skill_id == node.skill_id)
                    && edge
                        .relations
                        .iter()
                        .any(|relation| relation.source == "vector_similarity")
            })));
        assert!(graph.edges.iter().any(|edge| edge.nearest_fallback));
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
        assert_eq!(graph.excluded_unconnected_count, 2);
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
        assert_eq!(graph.excluded_unconnected_count, 1);
    }

    #[test]
    fn a_single_ready_agent_skill_remains_visible_without_an_edge() {
        let graph = build_skill_graph(
            &snapshot(vec![skill("claude", "claude-code"), skill("codex", "codex")]),
            "profile",
            &[vector("claude", vec![1.0, 0.0]), vector("codex", vec![0.0, 1.0])],
            &[],
            "claude-code",
        );
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].skill_id, "claude");
        assert!(graph.edges.is_empty());
        assert_eq!(graph.excluded_unconnected_count, 1);
    }
}
