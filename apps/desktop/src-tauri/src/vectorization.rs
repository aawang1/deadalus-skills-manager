use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

pub const INPUT_SCHEMA_VERSION: &str = "deadalus.embedding-input.v1";
pub const CHUNK_POLICY_VERSION: &str = "deadalus.chunk-policy.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorType {
    OverallFunction,
    Trigger,
    Workflow,
    Resource,
    General,
}

impl VectorType {
    pub const ALL: [Self; 5] = [
        Self::OverallFunction,
        Self::Trigger,
        Self::Workflow,
        Self::Resource,
        Self::General,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OverallFunction => "overall_function",
            Self::Trigger => "trigger",
            Self::Workflow => "workflow",
            Self::Resource => "resource",
            Self::General => "general",
        }
    }
}

impl fmt::Display for VectorType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for VectorType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "overall_function" => Ok(Self::OverallFunction),
            "trigger" => Ok(Self::Trigger),
            "workflow" => Ok(Self::Workflow),
            "resource" => Ok(Self::Resource),
            "general" => Ok(Self::General),
            _ => Err(format!("unknown vector type: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorLevel {
    Parent,
    Chunk,
}

impl VectorLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Parent => "parent",
            Self::Chunk => "chunk",
        }
    }
}

impl FromStr for VectorLevel {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "parent" => Ok(Self::Parent),
            "chunk" => Ok(Self::Chunk),
            _ => Err(format!("unknown vector level: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorStatus {
    Pending,
    Ready,
    Expired,
    Failed,
}

impl VectorStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Expired => "expired",
            Self::Failed => "failed",
        }
    }
}

impl FromStr for VectorStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "ready" => Ok(Self::Ready),
            "expired" => Ok(Self::Expired),
            "failed" => Ok(Self::Failed),
            _ => Err(format!("unknown vector status: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileStatus {
    Draft,
    Building,
    Ready,
    Active,
    Failed,
    Archived,
}

impl ProfileStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Building => "building",
            Self::Ready => "ready",
            Self::Active => "active",
            Self::Failed => "failed",
            Self::Archived => "archived",
        }
    }
}

impl FromStr for ProfileStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "building" => Ok(Self::Building),
            "ready" => Ok(Self::Ready),
            "active" => Ok(Self::Active),
            "failed" => Ok(Self::Failed),
            "archived" => Ok(Self::Archived),
            _ => Err(format!("unknown profile status: {value}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingProfile {
    pub profile_id: String,
    pub provider: String,
    pub model: String,
    pub model_version: String,
    pub dimensions: u32,
    pub input_schema_version: String,
    pub chunk_policy_version: String,
    pub tokenizer: Option<String>,
    pub credential_id: Option<String>,
    pub status: ProfileStatus,
    pub is_active: bool,
    pub created_at: i64,
    pub activated_at: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Incremental,
    FullRebuild,
    ProfileMigration,
    Validation,
}

impl JobKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Incremental => "incremental",
            Self::FullRebuild => "full_rebuild",
            Self::ProfileMigration => "profile_migration",
            Self::Validation => "validation",
        }
    }
}

impl FromStr for JobKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "incremental" => Ok(Self::Incremental),
            "full_rebuild" => Ok(Self::FullRebuild),
            "profile_migration" => Ok(Self::ProfileMigration),
            "validation" => Ok(Self::Validation),
            _ => Err(format!("unknown job kind: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl FromStr for JobStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "paused" => Ok(Self::Paused),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!("unknown job status: {value}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingJob {
    pub job_id: String,
    pub profile_id: String,
    pub kind: JobKind,
    pub status: JobStatus,
    pub total_items: u64,
    pub completed_items: u64,
    pub estimated_tokens: u64,
    pub actual_tokens: u64,
    pub estimated_cost_low: Option<f64>,
    pub estimated_cost_high: Option<f64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PreflightEstimate {
    pub complexity_level: u8,
    pub skill_count: u64,
    pub file_count: u64,
    pub analysis_file_count: u64,
    pub embedding_file_count: u64,
    pub embeddable_text_count: u64,
    pub parent_count: u64,
    pub chunk_count_low: u64,
    pub chunk_count_high: u64,
    pub analysis_tokens_low: u64,
    pub analysis_tokens_high: u64,
    pub embedding_tokens_low: u64,
    pub embedding_tokens_high: u64,
    pub estimated_seconds_low: u64,
    pub estimated_seconds_high: u64,
    pub estimated_cost_low: Option<f64>,
    pub estimated_cost_high: Option<f64>,
    pub provider: String,
    pub model: String,
    pub pricing_basis: Option<String>,
    pub estimated_at: i64,
    pub confidence: EstimateConfidence,
    pub missing_reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstimateConfidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipType {
    SimilarTo,
    OverlapsWith,
    ConflictsWith,
    DuplicateCandidate,
    Supersedes,
    DependsOn,
    ReadsReference,
    RunsScript,
    UsesAsset,
    LocatedIn,
}

impl RelationshipType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SimilarTo => "similar_to",
            Self::OverlapsWith => "overlaps_with",
            Self::ConflictsWith => "conflicts_with",
            Self::DuplicateCandidate => "duplicate_candidate",
            Self::Supersedes => "supersedes",
            Self::DependsOn => "depends_on",
            Self::ReadsReference => "reads_reference",
            Self::RunsScript => "runs_script",
            Self::UsesAsset => "uses_asset",
            Self::LocatedIn => "located_in",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipState {
    CarriedFact,
    HumanConfirmed,
    HumanRejected,
    HumanReverted,
    MigrationHint,
    Revalidated,
    Rejected,
    Conflict,
}

impl RelationshipState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CarriedFact => "carried_fact",
            Self::HumanConfirmed => "human_confirmed",
            Self::HumanRejected => "human_rejected",
            Self::HumanReverted => "human_reverted",
            Self::MigrationHint => "migration_hint",
            Self::Revalidated => "revalidated",
            Self::Rejected => "rejected",
            Self::Conflict => "conflict",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillRelationship {
    pub relation_id: String,
    pub source_skill_id: String,
    pub target_skill_id: String,
    pub relationship_type: RelationshipType,
    pub vector_type: Option<VectorType>,
    pub source_profile_id: Option<String>,
    pub target_profile_id: Option<String>,
    pub score: Option<f64>,
    pub state: RelationshipState,
    pub evidence: serde_json::Value,
    pub created_at: i64,
    pub validated_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationState {
    RuleDerived,
    LlmSynthetic,
    MachineConsensus,
    Conflict,
    HumanConfirmed,
    HumanRejected,
    HumanReverted,
}

impl ValidationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RuleDerived => "rule_derived",
            Self::LlmSynthetic => "llm_synthetic",
            Self::MachineConsensus => "machine_consensus",
            Self::Conflict => "conflict",
            Self::HumanConfirmed => "human_confirmed",
            Self::HumanRejected => "human_rejected",
            Self::HumanReverted => "human_reverted",
        }
    }
}

impl FromStr for ValidationState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "rule_derived" => Ok(Self::RuleDerived),
            "llm_synthetic" => Ok(Self::LlmSynthetic),
            "machine_consensus" => Ok(Self::MachineConsensus),
            "conflict" => Ok(Self::Conflict),
            "human_confirmed" => Ok(Self::HumanConfirmed),
            "human_rejected" => Ok(Self::HumanRejected),
            "human_reverted" => Ok(Self::HumanReverted),
            _ => Err(format!("unknown validation state: {value}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchEvidence {
    pub embedding_id: String,
    pub vector_type: VectorType,
    pub level: VectorLevel,
    pub chunk_id: Option<String>,
    pub raw_score: f32,
    pub expired: bool,
    pub heading_path: Option<String>,
    pub source_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub skill_id: String,
    pub score: f32,
    pub parent_score: f32,
    pub chunk_score: Option<f32>,
    pub expired: bool,
    pub matched_types: Vec<VectorType>,
    pub evidence: Vec<SearchEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StoredVector {
    pub embedding_id: String,
    pub profile_id: String,
    pub skill_id: String,
    pub vector_type: VectorType,
    pub level: VectorLevel,
    pub parent_embedding_id: Option<String>,
    pub resource_category: Option<String>,
    pub chunk_id: Option<String>,
    pub input_hash: String,
    pub vector: Vec<f32>,
    pub status: VectorStatus,
    pub heading_path: Option<String>,
    pub source_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StructuredParentInput {
    pub vector_type: VectorType,
    pub resource_category: Option<String>,
    pub title: String,
    pub fields: BTreeMap<String, Vec<String>>,
    pub source_headings: Vec<String>,
    pub schema_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedChunk {
    pub chunk_id: String,
    pub vector_type: VectorType,
    pub resource_category: Option<String>,
    pub relative_file_path: String,
    pub heading_path: String,
    pub semantic_role: String,
    pub local_anchor: String,
    pub text: String,
    pub token_estimate: usize,
    pub input_hash: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedInput {
    pub input_id: String,
    pub parent: StructuredParentInput,
    pub text: String,
    pub input_hash: String,
    pub chunks: Vec<GeneratedChunk>,
}

#[derive(Debug, Clone)]
pub struct InputGenerationConfig {
    pub max_parent_tokens: usize,
    pub max_chunk_tokens: usize,
    pub overlap_percent: usize,
}

impl Default for InputGenerationConfig {
    fn default() -> Self {
        Self {
            max_parent_tokens: 1_200,
            max_chunk_tokens: 800,
            overlap_percent: 10,
        }
    }
}

#[derive(Debug, Clone)]
struct MarkdownSection {
    heading_path: Vec<String>,
    text: String,
}

#[derive(Debug, Default, Deserialize)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
}

pub fn generate_inputs(
    relative_file_path: &str,
    markdown: &str,
    config: &InputGenerationConfig,
) -> Vec<GeneratedInput> {
    let (frontmatter, body) = split_frontmatter(markdown);
    let metadata = serde_yaml::from_str::<Frontmatter>(&frontmatter).unwrap_or_default();
    let sections = parse_sections(body);
    let fallback_name = relative_file_path
        .replace('\\', "/")
        .split('/')
        .rev()
        .nth(1)
        .unwrap_or("unnamed-skill")
        .to_string();
    let name = metadata.name.unwrap_or(fallback_name);
    let description = metadata.description.unwrap_or_default();

    let mut grouped: BTreeMap<(VectorType, Option<String>), Vec<MarkdownSection>> = BTreeMap::new();
    for section in sections {
        let (vector_type, resource_category) = classify_heading(&section.heading_path);
        if !section.text.trim().is_empty() {
            grouped
                .entry((vector_type, resource_category))
                .or_default()
                .push(section);
        }
    }

    let overall_sections = grouped
        .remove(&(VectorType::OverallFunction, None))
        .unwrap_or_default();
    let mut overall_fields = BTreeMap::new();
    overall_fields.insert("name".to_string(), vec![name.clone()]);
    if !description.is_empty() {
        overall_fields.insert("description".to_string(), vec![description.clone()]);
    }
    let overall_texts: Vec<String> = overall_sections
        .iter()
        .map(|section| section.text.clone())
        .collect();
    if !overall_texts.is_empty() {
        overall_fields.insert("core_capabilities".to_string(), overall_texts);
    }
    let overall_parent = StructuredParentInput {
        vector_type: VectorType::OverallFunction,
        resource_category: None,
        title: name.clone(),
        fields: overall_fields,
        source_headings: overall_sections
            .iter()
            .map(|section| section.heading_path.join(" > "))
            .collect(),
        schema_version: INPUT_SCHEMA_VERSION.to_string(),
    };
    let mut generated = vec![build_input(
        relative_file_path,
        overall_parent,
        &overall_sections,
        config,
    )];

    for ((vector_type, resource_category), category_sections) in grouped {
        let mut fields: BTreeMap<String, Vec<String>> = BTreeMap::new();
        fields.insert("name".to_string(), vec![name.clone()]);
        if !description.is_empty() {
            fields.insert("description".to_string(), vec![description.clone()]);
        }
        let field_name = match vector_type {
            VectorType::Trigger => "conditions",
            VectorType::Workflow => "steps",
            VectorType::Resource => "resources",
            VectorType::General => "unclassified",
            VectorType::OverallFunction => "core_capabilities",
        };
        fields.insert(
            field_name.to_string(),
            category_sections
                .iter()
                .map(|section| section.text.clone())
                .collect(),
        );
        let parent = StructuredParentInput {
            vector_type,
            resource_category,
            title: name.clone(),
            fields,
            source_headings: category_sections
                .iter()
                .map(|section| section.heading_path.join(" > "))
                .collect(),
            schema_version: INPUT_SCHEMA_VERSION.to_string(),
        };
        generated.push(build_input(
            relative_file_path,
            parent,
            &category_sections,
            config,
        ));
    }
    generated
}

fn build_input(
    relative_file_path: &str,
    parent: StructuredParentInput,
    sections: &[MarkdownSection],
    config: &InputGenerationConfig,
) -> GeneratedInput {
    let full_text = render_parent_text(&parent);
    let parent_is_oversized =
        estimate_tokens_bounded(&full_text, config.max_parent_tokens) > config.max_parent_tokens;
    // A Parent is an indexable structural summary, not a second copy of every
    // source section. Keep its text inside the active profile's input budget;
    // the complete detail remains available through the generated chunks and
    // the structured parent payload.
    let text = if parent_is_oversized {
        truncate_to_token_limit(&full_text, config.max_parent_tokens.max(1))
    } else {
        full_text
    };
    let input_hash = stable_hash(text.as_bytes());
    let identity = format!(
        "{}\0{}\0{}\0{}",
        parent.vector_type,
        parent.resource_category.as_deref().unwrap_or(""),
        relative_file_path.replace('\\', "/"),
        INPUT_SCHEMA_VERSION
    );
    let input_id = format!("input_{}", &stable_hash(identity.as_bytes())[..32]);
    let chunks = if parent_is_oversized {
        sections
            .iter()
            .flat_map(|section| {
                chunk_section(
                    relative_file_path,
                    parent.vector_type,
                    parent.resource_category.as_deref(),
                    section,
                    config.max_chunk_tokens.max(1),
                    config.overlap_percent.min(50),
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    GeneratedInput {
        input_id,
        parent,
        text,
        input_hash,
        chunks,
    }
}

fn truncate_to_token_limit(text: &str, max_tokens: usize) -> String {
    let spans = text_spans(text);
    if spans.is_empty() {
        return String::new();
    }
    let mut used = 0usize;
    let mut byte_end = 0usize;
    for span in spans {
        if used.saturating_add(span.tokens) > max_tokens {
            // A single long ASCII token can itself exceed the budget. Preserve
            // a bounded prefix instead of producing an empty Parent.
            if byte_end == 0 {
                byte_end = text
                    .char_indices()
                    .nth(max_tokens.saturating_mul(4))
                    .map(|(index, _)| index)
                    .unwrap_or(text.len());
            }
            break;
        }
        used = used.saturating_add(span.tokens);
        byte_end = span.end;
    }
    text[..byte_end].trim().to_string()
}

fn render_parent_text(parent: &StructuredParentInput) -> String {
    let mut lines = vec![
        format!("type: {}", parent.vector_type),
        format!("title: {}", parent.title.trim()),
    ];
    if let Some(category) = &parent.resource_category {
        lines.push(format!("resource_category: {category}"));
    }
    for (field, values) in &parent.fields {
        for value in values {
            let normalized = normalize_text(value);
            if !normalized.is_empty() {
                lines.push(format!("{field}: {normalized}"));
            }
        }
    }
    lines.join("\n")
}

fn split_frontmatter(markdown: &str) -> (String, &str) {
    let normalized = markdown.strip_prefix('\u{feff}').unwrap_or(markdown);
    let Some(after_start) = normalized.strip_prefix("---\n") else {
        return (String::new(), normalized);
    };
    let Some(end) = after_start.find("\n---") else {
        return (String::new(), normalized);
    };
    let body_offset = end + 4;
    let body = after_start
        .get(body_offset..)
        .unwrap_or_default()
        .strip_prefix('\n')
        .unwrap_or_else(|| after_start.get(body_offset..).unwrap_or_default());
    (after_start[..end].to_string(), body)
}

fn parse_sections(markdown: &str) -> Vec<MarkdownSection> {
    let parser = Parser::new_ext(markdown, Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS);
    let mut sections = Vec::new();
    let mut heading_stack: Vec<String> = Vec::new();
    let mut current_heading = String::new();
    let mut current_level = 0usize;
    let mut current_text = String::new();
    let mut in_heading = false;

    let flush = |sections: &mut Vec<MarkdownSection>, headings: &[String], text: &mut String| {
        let normalized = normalize_text(text);
        if !normalized.is_empty() {
            sections.push(MarkdownSection {
                heading_path: headings.to_vec(),
                text: normalized,
            });
        }
        text.clear();
    };

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut sections, &heading_stack, &mut current_text);
                in_heading = true;
                current_heading.clear();
                current_level = level as usize;
            }
            Event::End(TagEnd::Heading(_)) => {
                heading_stack.truncate(current_level.saturating_sub(1));
                heading_stack.push(normalize_text(&current_heading));
                in_heading = false;
            }
            Event::Text(value) | Event::Code(value) => {
                if in_heading {
                    current_heading.push_str(&value);
                } else {
                    current_text.push_str(&value);
                    current_text.push(' ');
                }
            }
            Event::SoftBreak | Event::HardBreak => current_text.push('\n'),
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Item
                | TagEnd::CodeBlock
                | TagEnd::TableRow
                | TagEnd::BlockQuote(_),
            ) => current_text.push('\n'),
            _ => {}
        }
    }
    flush(&mut sections, &heading_stack, &mut current_text);
    sections
}

fn classify_heading(path: &[String]) -> (VectorType, Option<String>) {
    let heading = path
        .last()
        .map(|part| part.to_lowercase())
        .unwrap_or_default();
    if contains_alias(
        &heading,
        &[
            "trigger",
            "when to use",
            "use when",
            "do not use",
            "触发",
            "使用场景",
            "何时使用",
            "适用场景",
        ],
    ) {
        return (VectorType::Trigger, None);
    }
    if contains_alias(
        &heading,
        &[
            "workflow",
            "steps",
            "instructions",
            "process",
            "工作流",
            "步骤",
            "流程",
            "操作",
        ],
    ) {
        return (VectorType::Workflow, None);
    }
    if contains_alias(
        &heading,
        &[
            "resource",
            "resources",
            "script",
            "reference",
            "example",
            "tool",
            "dependency",
            "external service",
            "资源",
            "脚本",
            "参考",
            "示例",
            "工具",
            "依赖",
            "外部服务",
        ],
    ) {
        return (
            VectorType::Resource,
            Some(resource_category(&heading).to_string()),
        );
    }
    if contains_alias(
        &heading,
        &[
            "overview",
            "summary",
            "purpose",
            "function",
            "capabilities",
            "概述",
            "简介",
            "功能",
            "能力",
            "用途",
        ],
    ) {
        return (VectorType::OverallFunction, None);
    }
    (VectorType::General, None)
}

fn contains_alias(value: &str, aliases: &[&str]) -> bool {
    aliases.iter().any(|alias| value.contains(alias))
}

fn resource_category(heading: &str) -> &'static str {
    if contains_alias(heading, &["script", "脚本"]) {
        "scripts"
    } else if contains_alias(heading, &["reference", "参考"]) {
        "references"
    } else if contains_alias(heading, &["example", "示例"]) {
        "examples"
    } else if contains_alias(heading, &["external service", "外部服务"]) {
        "external_services"
    } else if contains_alias(heading, &["tool", "dependency", "工具", "依赖"]) {
        "tools_dependencies"
    } else {
        "resources"
    }
}

fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

#[derive(Debug, Clone)]
struct TextSpan {
    start: usize,
    end: usize,
    tokens: usize,
}

fn text_spans(text: &str) -> Vec<TextSpan> {
    let mut spans = Vec::new();
    let mut word_start = None;
    let mut word_chars = 0usize;

    let flush_word =
        |end: usize, spans: &mut Vec<TextSpan>, start: &mut Option<usize>, chars: &mut usize| {
            if let Some(word_start) = start.take() {
                spans.push(TextSpan {
                    start: word_start,
                    end,
                    tokens: (*chars).div_ceil(4).max(1),
                });
                *chars = 0;
            }
        };

    for (index, character) in text.char_indices() {
        if character.is_ascii_alphanumeric() || character == '_' {
            word_start.get_or_insert(index);
            word_chars += 1;
            continue;
        }
        flush_word(index, &mut spans, &mut word_start, &mut word_chars);
        let end = index + character.len_utf8();
        if !character.is_whitespace() {
            spans.push(TextSpan {
                start: index,
                end,
                tokens: 1,
            });
        }
    }
    flush_word(text.len(), &mut spans, &mut word_start, &mut word_chars);
    spans
}

pub fn estimate_tokens(text: &str) -> usize {
    text_spans(text).iter().map(|span| span.tokens).sum()
}

pub fn estimate_tokens_bounded(text: &str, limit: usize) -> usize {
    let mut total = 0usize;
    for span in text_spans(text) {
        total = total.saturating_add(span.tokens);
        if total > limit {
            return limit.saturating_add(1);
        }
    }
    total
}

fn chunk_section(
    relative_file_path: &str,
    vector_type: VectorType,
    resource_category: Option<&str>,
    section: &MarkdownSection,
    max_tokens: usize,
    overlap_percent: usize,
) -> Vec<GeneratedChunk> {
    let spans = text_spans(&section.text);
    if spans.is_empty() {
        return Vec::new();
    }
    let heading_path = if section.heading_path.is_empty() {
        "root".to_string()
    } else {
        section.heading_path.join(" > ")
    };
    let role = match vector_type {
        VectorType::OverallFunction => "overall_function",
        VectorType::Trigger => "trigger",
        VectorType::Workflow => "workflow",
        VectorType::Resource => "resource",
        VectorType::General => "unclassified",
    };
    let wrapper = format!("type: {vector_type}\nheading: {heading_path}\nrole: {role}\ntext:");
    let content_budget = max_tokens.saturating_sub(estimate_tokens(&wrapper)).max(1);
    let overlap_tokens = (content_budget * overlap_percent / 100).max(1);
    let mut ranges = Vec::new();
    let mut start = 0usize;
    while start < spans.len() {
        let mut end = start;
        let mut tokens = 0usize;
        while end < spans.len() && tokens.saturating_add(spans[end].tokens) <= content_budget {
            tokens += spans[end].tokens;
            end += 1;
        }
        if end == start {
            end += 1;
        }
        ranges.push((start, end));
        if end == spans.len() {
            break;
        }
        let mut overlap_start = end;
        let mut overlap = 0usize;
        while overlap_start > start && overlap < overlap_tokens {
            overlap_start -= 1;
            overlap += spans[overlap_start].tokens;
        }
        start = overlap_start.max(start + 1);
    }

    ranges
        .into_iter()
        .map(|(start, end)| {
            let byte_start = spans[start].start;
            let byte_end = spans[end - 1].end;
            let chunk_text = section.text[byte_start..byte_end].trim().to_string();
            let anchor_source = chunk_text.chars().take(64).collect::<String>();
            let local_anchor = format!(
                "{}-{}",
                slug(&heading_path),
                &stable_hash(anchor_source.as_bytes())[..12]
            );
            stable_chunk(
                relative_file_path,
                vector_type,
                resource_category,
                &heading_path,
                role,
                &local_anchor,
                chunk_text,
            )
        })
        .collect()
}

fn stable_chunk(
    relative_file_path: &str,
    vector_type: VectorType,
    resource_category: Option<&str>,
    heading_path: &str,
    semantic_role: &str,
    local_anchor: &str,
    text: String,
) -> GeneratedChunk {
    let normalized_path = relative_file_path.replace('\\', "/");
    let identity = format!(
        "{}\0{}\0{}\0{}\0{}",
        vector_type, normalized_path, heading_path, semantic_role, local_anchor
    );
    let content_hash = stable_hash(text.as_bytes());
    let input_text = format!(
        "type: {vector_type}\nheading: {heading_path}\nrole: {semantic_role}\ntext: {text}"
    );
    GeneratedChunk {
        chunk_id: format!("chunk_{}", &stable_hash(identity.as_bytes())[..32]),
        vector_type,
        resource_category: resource_category.map(str::to_string),
        relative_file_path: normalized_path,
        heading_path: heading_path.to_string(),
        semantic_role: semantic_role.to_string(),
        local_anchor: local_anchor.to_string(),
        token_estimate: estimate_tokens(&input_text),
        input_hash: stable_hash(input_text.as_bytes()),
        content_hash,
        text: input_text,
    }
}

fn slug(value: &str) -> String {
    let slug = value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    slug.split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

pub fn stable_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn input_types(inputs: &[GeneratedInput]) -> BTreeSet<VectorType> {
    inputs
        .iter()
        .map(|input| input.parent.vector_type)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKDOWN: &str = r#"---
name: pdf-reader
description: Read and summarize PDF files
---
# Overview
Extract text and metadata from PDFs.

## When to use
Use when the user asks to inspect a PDF. Do not use for spreadsheets.

## Workflow
1. Validate the file.
2. Extract text.
3. Return a cited summary.

## Scripts
The `extract.py` script uses Python and pypdf.

## Notes
Preserve page numbers in every citation.
"#;

    #[test]
    fn parser_generates_expected_independent_parent_types() {
        let inputs = generate_inputs("pdf-reader/SKILL.md", MARKDOWN, &Default::default());
        let types = input_types(&inputs);
        assert!(types.contains(&VectorType::OverallFunction));
        assert!(types.contains(&VectorType::Trigger));
        assert!(types.contains(&VectorType::Workflow));
        assert!(types.contains(&VectorType::Resource));
        assert!(types.contains(&VectorType::General));
        let resource = inputs
            .iter()
            .find(|input| input.parent.vector_type == VectorType::Resource)
            .expect("resource input should exist");
        assert_eq!(
            resource.parent.resource_category.as_deref(),
            Some("scripts")
        );
    }

    #[test]
    fn generated_ids_and_hashes_are_stable() {
        let config = InputGenerationConfig {
            max_parent_tokens: 8,
            max_chunk_tokens: 5,
            overlap_percent: 10,
        };
        let first = generate_inputs("pdf-reader/SKILL.md", MARKDOWN, &config);
        let second = generate_inputs("pdf-reader/SKILL.md", MARKDOWN, &config);
        assert_eq!(first, second);
        assert!(first.iter().all(|input| input.input_hash.len() == 64));
        assert!(first
            .iter()
            .flat_map(|input| &input.chunks)
            .all(|chunk| chunk.chunk_id.starts_with("chunk_")));
    }

    #[test]
    fn long_text_chunks_are_bounded_and_overlap() {
        let repeated = (0..120)
            .map(|index| format!("step{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        let markdown = format!("# Workflow\n{repeated}");
        let config = InputGenerationConfig {
            max_parent_tokens: 10,
            max_chunk_tokens: 20,
            overlap_percent: 10,
        };
        let inputs = generate_inputs("long/SKILL.md", &markdown, &config);
        let workflow = inputs
            .iter()
            .find(|input| input.parent.vector_type == VectorType::Workflow)
            .expect("workflow should exist");
        assert!(estimate_tokens(&workflow.text) <= config.max_parent_tokens);
        assert!(workflow.chunks.len() > 1);
        assert!(workflow
            .chunks
            .iter()
            .all(|chunk| chunk.token_estimate <= 30));
        let first_words: BTreeSet<_> = workflow.chunks[0].text.split_whitespace().collect();
        let second_words: BTreeSet<_> = workflow.chunks[1].text.split_whitespace().collect();
        assert!(!first_words.is_disjoint(&second_words));
    }

    #[test]
    fn bounded_estimator_stops_after_limit() {
        assert_eq!(estimate_tokens_bounded("one two three four", 2), 3);
        assert!(estimate_tokens("你好 world") >= 3);
    }

    #[test]
    fn oversized_unbroken_parent_is_non_empty_and_bounded() {
        let markdown = format!("# Notes\n{}", "a".repeat(100_000));
        let config = InputGenerationConfig {
            max_parent_tokens: 100,
            max_chunk_tokens: 80,
            overlap_percent: 10,
        };
        let inputs = generate_inputs("large/SKILL.md", &markdown, &config);
        let general = inputs
            .iter()
            .find(|input| input.parent.vector_type == VectorType::General)
            .expect("general input should exist");
        assert!(!general.text.is_empty());
        assert!(estimate_tokens(&general.text) <= config.max_parent_tokens);
        assert!(!general.chunks.is_empty());
    }
}
