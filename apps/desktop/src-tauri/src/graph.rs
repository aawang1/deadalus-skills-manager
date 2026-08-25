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

    let excluded_unconnected_count = 0;
    let (clusters, cluster_by_skill, centrality_by_skill) =
        build_clusters(&ready_skills, &similarities);
    let superseded_ids = confirmed_superseded_ids(&relevant_relationships);
    let nodes = ready_skills
        .values()
        .map(|(skill, _)| {
            graph_node(
                skill,
                cluster_by_skill.get(&skill.skill_id).cloned(),
                *centrality_by_skill.get(&skill.skill_id).unwrap_or(&0.0),
                superseded_ids.contains(&skill.skill_id),
            )
        })
        .collect::<Vec<_>>();
    let node_ids = nodes
        .iter()
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
) -> (
    Vec<SkillGraphCluster>,
    HashMap<String, String>,
    HashMap<String, f32>,
) {
    let mut adjacency = ready_skills
        .keys()
        .map(|id| (id.clone(), Vec::<String>::new()))
        .collect::<HashMap<_, _>>();
    for pair in similarities
        .iter()
        .filter(|pair| pair.score >= CLUSTER_SIMILARITY_THRESHOLD)
    {
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
        let skills = members
            .iter()
            .filter_map(|id| ready_skills.get(id).map(|(skill, _)| *skill))
            .collect::<Vec<_>>();
        clusters.push(SkillGraphCluster {
            cluster_id,
            name: cluster_name(&skills),
            summary: cluster_summary(&skills),
            member_skill_ids: members,
            core_skill_ids: ranked.into_iter().take(core_count).collect(),
            peripheral: false,
        });
    }
    clusters.sort_by(|left, right| left.cluster_id.cmp(&right.cluster_id));
    (clusters, cluster_by_skill, centrality_by_skill)
}

fn cluster_name(skills: &[&CanonicalSkill]) -> String {
    let stop_words = [
        "skill", "skills", "tool", "agent", "the", "and", "for", "with",
    ];
    let mut counts = BTreeMap::<String, usize>::new();
    for skill in skills {
        for token in skill
            .name
            .split(|character: char| !character.is_alphanumeric())
            .filter(|token| token.chars().count() >= 2)
        {
            let normalized = token.to_lowercase();
            if !stop_words.contains(&normalized.as_str()) {
                *counts.entry(normalized).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
        .map(|(token, _)| token)
        .or_else(|| skills.first().map(|skill| skill.name.clone()))
        .unwrap_or_else(|| "功能集群".to_string())
}

fn cluster_summary(skills: &[&CanonicalSkill]) -> String {
    let descriptions = skills
        .iter()
        .filter_map(|skill| skill.description.as_deref())
        .filter(|description| !description.trim().is_empty())
        .take(3)
        .collect::<Vec<_>>();
    if descriptions.is_empty() {
        format!("包含 {} 个功能相近的 Skills。", skills.len())
    } else {
        let joined = descriptions.join("；");
        let summary = joined.chars().take(180).collect::<String>();
        format!("{} 个 Skills：{}", skills.len(), summary)
    }
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
            "{}:{}:{}:{}:{}:{}",
            node.skill_id,
            node.name,
            node.description.as_deref().unwrap_or_default(),
            node.path,
            node.enabled_agents.join(","),
            vector_version,
        ));
    }
    for cluster in clusters {
        parts.push(format!(
            "{}:{}:{}:{}:{}",
            cluster.cluster_id,
            cluster.name,
            cluster.summary,
            cluster.member_skill_ids.join(","),
            cluster.core_skill_ids.join(","),
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
        parts.push(format!("{}:{}", node.skill_id, vector_version));
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
