pub mod adaptive;
pub mod application;
pub mod database;
pub mod graph;
pub mod pipeline;
pub mod validation;
pub mod vectorization;

use keyring::Entry;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{mpsc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, State};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};
use uuid::Uuid;

use adaptive::{
    initial_policy, propose_policy, AdaptivePolicyState, AdaptiveProposal,
    ADAPTIVE_POLICY_SCHEMA_VERSION,
};
use application::{
    migrate_profile_relations, retrieve_semantic_results, AnalysisPhase, AnalysisProvider,
    AnalysisRequest, QueryVectors, RetrievalConfig,
};
use database::{
    AdaptiveHistoryRecord, AgentSkillState, CanonicalFile, CanonicalSkill, CanonicalSnapshot,
    Database, FeedbackEventRecord, LocalValidationMutation, ValidationRunRecord,
    ValidationSampleRecord,
};
use graph::{build_skill_graph, empty_graph, SkillGraphSnapshot};
use pipeline::{
    compare_analysis, estimate_preflight, prepare_skill_inputs, rule_result, EmbeddingJobContext,
    EmbeddingProvider, IndexDiff, IndexSyncStatus, JobStrategy, PreparedProfileChange,
    ProfileChangeRequest, ProgressEvent, RemoteAnalysisProvider, RemoteEmbeddingProvider,
    ReqwestJsonTransport, ToastMessage,
};
use validation::{
    compute_metrics, generate_samples, split_for_family, EvaluatedSample, FeedbackAction,
    FeedbackRequest, ValidationTaskType, LOCAL_DATASET_SCHEMA_VERSION,
};
use vectorization::{
    stable_hash, EmbeddingJob, EmbeddingProfile, InputGenerationConfig, JobStatus, ProfileStatus,
    RelationshipState, RelationshipType, SearchResult, SkillRelationship, StoredVector,
    ValidationState, VectorLevel, VectorStatus, VectorType, CHUNK_POLICY_VERSION,
    INPUT_SCHEMA_VERSION,
};

const CREDENTIAL_SERVICE: &str = "app.deadalus.desktop";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const QWEN_EMBEDDING_ENDPOINT: &str =
    "https://dashscope.aliyuncs.com/compatible-mode/v1/embeddings";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiKeyMetadata {
    id: String,
    provider: String,
    masked_key: String,
    saved_at: u64,
    #[serde(default)]
    is_agent_active: bool,
    #[serde(default)]
    is_embedding_active: bool,
    #[serde(default)]
    agent_model: Option<String>,
    #[serde(default, rename = "isActive", skip_serializing)]
    legacy_is_active: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CredentialPurpose {
    Agent,
    Embedding,
}

impl CredentialPurpose {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Embedding => "embedding",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ProviderModel {
    id: String,
    display_name: String,
    owned_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ProviderModelsResponse {
    credential_id: String,
    provider: String,
    models: Vec<ProviderModel>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateEmbeddingProfileRequest {
    provider: String,
    model: String,
    version: String,
    dimensions: u32,
    credential_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct EmbeddingProfileDefaults {
    provider: String,
    model: String,
    version: String,
    dimensions: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EmbeddingProfileSettings {
    profile: EmbeddingProfile,
    defaults: EmbeddingProfileDefaults,
    credential_ready: bool,
    adaptive_policy: AdaptivePolicyState,
    adaptive_history: Vec<AdaptiveHistoryRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
}

const MANAGED_SKILL_ORIGIN_FILE: &str = ".deadalus-origin.json";
const MANAGED_SKILL_HISTORY_DIR: &str = ".history";
const SKILL_IDENTITY_V3_MARKER: &str = "skill-identity-v3.migrated";
const BUILT_IN_CLASSIFICATION_V3_MARKER: &str = "built-in-classification-v3.migrated";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedSkillOrigin {
    source_path: String,
    managed_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstalledSkill {
    #[serde(default)]
    skill_id: String,
    name: String,
    description: Option<String>,
    path: String,
    source_path: String,
    scope: String,
    is_built_in: bool,
    enabled_agents: Vec<String>,
    #[serde(default)]
    disabled_agents: Vec<String>,
    in_library: bool,
    library_path: Option<String>,
    #[serde(default)]
    backup_suppressed: bool,
    #[serde(skip_serializing, default)]
    managed_origin: Option<String>,
    #[serde(skip_serializing, default)]
    content_hash: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillActionPlan {
    skill_id: String,
    skill_name: String,
    view_id: String,
    action: String,
    allowed: bool,
    reason: Option<String>,
    target_paths: Vec<String>,
    affected_agents: Vec<String>,
    shared_installation: bool,
    plugin_operation: bool,
    symbolic_link_only: bool,
    keeps_library_copy: bool,
    keeps_other_agents: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentSkillsResponse {
    agent: String,
    official_documentation: String,
    searched_paths: Vec<String>,
    skills: Vec<InstalledSkill>,
    warnings: Vec<String>,
}

struct LibraryWatcher {
    _watcher: Mutex<RecommendedWatcher>,
}

fn unix_timestamp() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .map_err(|error| format!("无法读取系统时间：{error}"))
}

fn stable_skill_id(logical_identity: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"deadalus.canonical-skill.v2\0");
    hasher.update(logical_identity.as_bytes());
    let digest = hasher.finalize();
    let encoded = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("skill_{}", &encoded[..32])
}

fn credential_id(provider: &str) -> Result<String, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| format!("{provider}-{}-{}", value.as_nanos(), std::process::id()))
        .map_err(|error| format!("无法生成凭据标识：{error}"))
}

fn metadata_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("api-key-metadata.json"))
        .map_err(|error| format!("无法确定应用数据目录：{error}"))
}

fn read_key_metadata(app: &AppHandle) -> Result<Vec<ApiKeyMetadata>, String> {
    let path = metadata_path(app)?;
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content =
        fs::read_to_string(&path).map_err(|error| format!("无法读取 Key 元数据：{error}"))?;
    let (keys, migrated) = parse_key_metadata(&content)?;
    if migrated {
        write_key_metadata(app, &keys)?;
    }
    Ok(keys)
}

fn parse_key_metadata(content: &str) -> Result<(Vec<ApiKeyMetadata>, bool), String> {
    let mut keys: Vec<ApiKeyMetadata> =
        serde_json::from_str(content).map_err(|error| format!("Key 元数据格式无效：{error}"))?;
    let mut migrated = false;
    for key in &mut keys {
        if let Some(was_active) = key.legacy_is_active.take() {
            migrated = true;
            if was_active && !key.is_agent_active && !key.is_embedding_active {
                key.is_agent_active = true;
                key.is_embedding_active = is_embedding_provider(&key.provider);
            }
        }
    }
    Ok((keys, migrated))
}

fn is_embedding_provider(provider: &str) -> bool {
    matches!(provider, "openai" | "qwen")
}

fn write_key_metadata(app: &AppHandle, keys: &[ApiKeyMetadata]) -> Result<(), String> {
    let path = metadata_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法创建应用数据目录：{error}"))?;
    }

    let content = serde_json::to_string_pretty(keys)
        .map_err(|error| format!("无法序列化 Key 元数据：{error}"))?;
    fs::write(path, content).map_err(|error| format!("无法保存 Key 元数据：{error}"))
}

fn mask_key(api_key: &str) -> String {
    let suffix: String = api_key
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("••••••••{suffix}")
}

async fn validate_api_key(provider: &str, api_key: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| format!("无法创建验证客户端：{error}"))?;

    let response = match provider {
        "openai" => {
            client
                .get("https://api.openai.com/v1/models")
                .header(AUTHORIZATION, format!("Bearer {api_key}"))
                .send()
                .await
        }
        "anthropic" => {
            client
                .get("https://api.anthropic.com/v1/models?limit=1")
                .header("x-api-key", api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header(CONTENT_TYPE, "application/json")
                .send()
                .await
        }
        "deepseek" => {
            client
                .get("https://api.deepseek.com/models")
                .header(AUTHORIZATION, format!("Bearer {api_key}"))
                .send()
                .await
        }
        "qwen" => {
            client
                .post(QWEN_EMBEDDING_ENDPOINT)
                .header(AUTHORIZATION, format!("Bearer {api_key}"))
                .header(CONTENT_TYPE, "application/json")
                .json(&serde_json::json!({
                    "model": "text-embedding-v4",
                    "input": "Deadalus credential validation",
                    "dimensions": 64
                }))
                .send()
                .await
        }
        _ => return Err("暂不支持该 API Provider".to_string()),
    }
    .map_err(|error| format!("无法连接 Provider：{error}"))?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "Provider 拒绝了该 Key（HTTP {}）",
            response.status().as_u16()
        ))
    }
}

#[tauri::command]
fn list_api_keys(app: AppHandle) -> Result<Vec<ApiKeyMetadata>, String> {
    read_key_metadata(&app)
}

#[tauri::command]
async fn save_api_key(
    app: AppHandle,
    provider: String,
    api_key: String,
) -> Result<ApiKeyMetadata, String> {
    let provider = provider.trim().to_ascii_lowercase();
    let api_key = api_key.trim();

    if api_key.len() < 12 {
        return Err("API Key 长度无效".to_string());
    }

    validate_api_key(&provider, api_key).await?;

    let saved_at = unix_timestamp()?;
    let id = credential_id(&provider)?;
    let credential = Entry::new(CREDENTIAL_SERVICE, &id)
        .map_err(|error| format!("无法访问系统凭据库：{error}"))?;
    credential
        .set_password(api_key)
        .map_err(|error| format!("无法保存到 Windows Credential Manager：{error}"))?;

    let metadata = ApiKeyMetadata {
        id,
        provider,
        masked_key: mask_key(api_key),
        saved_at,
        is_agent_active: false,
        is_embedding_active: false,
        agent_model: None,
        legacy_is_active: None,
    };
    let mut keys = read_key_metadata(&app)?;
    keys.push(metadata.clone());

    if let Err(error) = write_key_metadata(&app, &keys) {
        let _ = credential.delete_credential();
        return Err(error);
    }

    Ok(metadata)
}

#[tauri::command]
fn activate_api_key_for_purpose(
    app: AppHandle,
    database: State<'_, Database>,
    id: String,
    purpose: CredentialPurpose,
) -> Result<Vec<ApiKeyMetadata>, String> {
    activate_api_key_for_purpose_impl(&app, database.inner(), &id, purpose)
}

fn activate_api_key_for_purpose_impl(
    app: &AppHandle,
    database: &Database,
    id: &str,
    purpose: CredentialPurpose,
) -> Result<Vec<ApiKeyMetadata>, String> {
    let mut keys = read_key_metadata(app)?;
    let provider = keys
        .iter()
        .find(|key| key.id == id)
        .map(|key| key.provider.clone())
        .ok_or_else(|| "未找到该 API Key".to_string())?;
    read_credential_secret(id)?;
    activate_metadata_for_purpose(&mut keys, id, purpose)?;
    let original_keys = read_key_metadata(app)?;
    write_key_metadata(app, &keys)?;
    let updated_at =
        i64::try_from(unix_timestamp()?).map_err(|_| "凭据更新时间超出支持范围".to_string())?;
    if let Err(error) =
        database.activate_credential_binding(id, purpose.as_str(), &provider, updated_at)
    {
        let _ = write_key_metadata(app, &original_keys);
        return Err(format!("无法保存凭据用途绑定：{error}"));
    }
    Ok(keys)
}

fn activate_metadata_for_purpose(
    keys: &mut [ApiKeyMetadata],
    id: &str,
    purpose: CredentialPurpose,
) -> Result<(), String> {
    let selected = keys
        .iter()
        .find(|key| key.id == id)
        .ok_or_else(|| "未找到该 API Key".to_string())?;
    if purpose == CredentialPurpose::Embedding && !is_embedding_provider(&selected.provider) {
        return Err("该 Provider 当前不支持 Embedding 用途".to_string());
    }
    for key in keys {
        match purpose {
            CredentialPurpose::Agent => key.is_agent_active = key.id == id,
            CredentialPurpose::Embedding => key.is_embedding_active = key.id == id,
        }
    }
    Ok(())
}

fn read_credential_secret(id: &str) -> Result<String, String> {
    Entry::new(CREDENTIAL_SERVICE, id)
        .map_err(|error| format!("无法访问系统凭据库：{error}"))?
        .get_password()
        .map_err(|error| format!("系统凭据不存在或无法读取：{error}"))
}

#[tauri::command]
fn activate_api_key(
    app: AppHandle,
    database: State<'_, Database>,
    id: String,
) -> Result<Vec<ApiKeyMetadata>, String> {
    activate_api_key_for_purpose_impl(&app, database.inner(), &id, CredentialPurpose::Agent)
}

#[tauri::command]
fn delete_api_key(
    app: AppHandle,
    database: State<'_, Database>,
    id: String,
) -> Result<Vec<ApiKeyMetadata>, String> {
    let original_keys = read_key_metadata(&app)?;
    if !original_keys.iter().any(|key| key.id == id) {
        return Err("未找到该 API Key".to_string());
    }

    let mut remaining_keys = original_keys.clone();
    remaining_keys.retain(|key| key.id != id);
    write_key_metadata(&app, &remaining_keys)?;

    let credential = Entry::new(CREDENTIAL_SERVICE, &id)
        .map_err(|error| format!("无法访问系统凭据库：{error}"))?;
    if let Err(error) = credential.delete_credential() {
        let _ = write_key_metadata(&app, &original_keys);
        return Err(format!("无法从 Windows Credential Manager 删除：{error}"));
    }
    database
        .detach_credential(&id)
        .map_err(|error| format!("凭据已删除，但无法清理用途/Profile 引用：{error}"))?;

    Ok(remaining_keys)
}

#[tauri::command]
fn set_agent_model(
    app: AppHandle,
    id: String,
    model: String,
) -> Result<Vec<ApiKeyMetadata>, String> {
    let model = model.trim();
    if model.is_empty() || model.len() > 200 {
        return Err("Agent 模型名称无效".to_string());
    }
    let mut keys = read_key_metadata(&app)?;
    let key = keys
        .iter_mut()
        .find(|key| key.id == id)
        .ok_or_else(|| "未找到该 API Key".to_string())?;
    key.agent_model = Some(model.to_string());
    write_key_metadata(&app, &keys)?;
    Ok(keys)
}

#[tauri::command]
async fn list_provider_models(
    app: AppHandle,
    id: String,
) -> Result<ProviderModelsResponse, String> {
    let keys = read_key_metadata(&app)?;
    let key = keys
        .iter()
        .find(|key| key.id == id)
        .ok_or_else(|| "未找到该 API Key".to_string())?;
    let api_key = read_credential_secret(&id)?;
    let models = fetch_provider_models(&key.provider, &api_key).await?;
    Ok(ProviderModelsResponse {
        credential_id: id,
        provider: key.provider.clone(),
        models,
    })
}

async fn fetch_provider_models(
    provider: &str,
    api_key: &str,
) -> Result<Vec<ProviderModel>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| format!("无法创建模型列表客户端：{error}"))?;
    let request = match provider {
        "openai" => client
            .get("https://api.openai.com/v1/models")
            .header(AUTHORIZATION, format!("Bearer {api_key}")),
        "anthropic" => client
            .get("https://api.anthropic.com/v1/models?limit=100")
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION),
        "deepseek" => client
            .get("https://api.deepseek.com/models")
            .header(AUTHORIZATION, format!("Bearer {api_key}")),
        "qwen" => client
            .get("https://dashscope.aliyuncs.com/compatible-mode/v1/models")
            .header(AUTHORIZATION, format!("Bearer {api_key}")),
        _ => return Err("暂不支持该 API Provider".to_string()),
    };
    let response = request
        .header(CONTENT_TYPE, "application/json")
        .send()
        .await
        .map_err(|error| format!("无法读取 Provider 模型列表：{error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("无法读取 Provider 模型响应：{error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Provider 拒绝了模型列表请求（HTTP {}）",
            status.as_u16()
        ));
    }
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| format!("Provider 模型响应不是 JSON：{error}"))?;
    parse_provider_models(&value)
}

fn parse_provider_models(value: &serde_json::Value) -> Result<Vec<ProviderModel>, String> {
    let items = value
        .as_array()
        .or_else(|| value.get("data").and_then(serde_json::Value::as_array))
        .or_else(|| value.get("models").and_then(serde_json::Value::as_array))
        .ok_or_else(|| "Provider 模型响应缺少 data/models 数组".to_string())?;
    let mut models = items
        .iter()
        .filter_map(|item| {
            let id = item
                .get("id")
                .or_else(|| item.get("model"))
                .or_else(|| item.get("name"))
                .and_then(serde_json::Value::as_str)?
                .trim();
            if id.is_empty() {
                return None;
            }
            let display_name = item
                .get("display_name")
                .or_else(|| item.get("displayName"))
                .or_else(|| item.get("name"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or(id)
                .to_string();
            let owned_by = item
                .get("owned_by")
                .or_else(|| item.get("ownedBy"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            Some(ProviderModel {
                id: id.to_string(),
                display_name,
                owned_by,
            })
        })
        .collect::<Vec<_>>();
    models.sort_by(|left, right| left.id.cmp(&right.id));
    models.dedup_by(|left, right| left.id == right.id);
    if models.is_empty() {
        return Err("Provider 模型响应中没有有效模型".to_string());
    }
    Ok(models)
}

fn embedding_profile_defaults(provider: &str) -> Result<EmbeddingProfileDefaults, String> {
    match provider {
        "openai" => Ok(EmbeddingProfileDefaults {
            provider: "openai".to_string(),
            model: "text-embedding-3-small".to_string(),
            version: "1".to_string(),
            dimensions: 1536,
        }),
        "qwen" => Ok(EmbeddingProfileDefaults {
            provider: "qwen".to_string(),
            model: "text-embedding-v4".to_string(),
            version: "1".to_string(),
            dimensions: 1024,
        }),
        _ => Err("该 Provider 当前不支持 Embedding Profile".to_string()),
    }
}

fn validate_embedding_credential(
    provider: &str,
    credential_id: &str,
    keys: &[ApiKeyMetadata],
) -> Result<(), String> {
    validate_embedding_credential_by_id(provider, credential_id, keys, true)
}

fn validate_embedding_credential_by_id(
    provider: &str,
    credential_id: &str,
    keys: &[ApiKeyMetadata],
    require_active: bool,
) -> Result<(), String> {
    let credential = keys
        .iter()
        .find(|key| key.id == credential_id)
        .ok_or_else(|| "Embedding Profile 引用的凭据不存在".to_string())?;
    if require_active && !credential.is_embedding_active {
        return Err("该凭据未启用 Embedding 用途".to_string());
    }
    if credential.provider != provider {
        return Err("Embedding Profile 与凭据 Provider 不匹配".to_string());
    }
    if !is_embedding_provider(provider) {
        return Err("该 Provider 当前不支持 Embedding Profile".to_string());
    }
    Ok(())
}

#[tauri::command]
fn get_embedding_profile_defaults(provider: String) -> Result<EmbeddingProfileDefaults, String> {
    embedding_profile_defaults(&provider.trim().to_ascii_lowercase())
}

#[tauri::command]
fn list_embedding_profiles(database: State<'_, Database>) -> Result<Vec<EmbeddingProfile>, String> {
    database
        .list_profiles()
        .map_err(|error| format!("无法读取 Embedding Profiles：{error}"))
}

#[tauri::command]
fn delete_embedding_profile(
    database: State<'_, Database>,
    profile_id: String,
) -> Result<bool, String> {
    database.delete_profile(&profile_id).map_err(|error| {
        let detail = error.to_string();
        if detail.contains("delete_active") {
            "不能删除当前活动 Profile；请先激活其他 Ready Profile。".to_string()
        } else if detail.contains("delete_with_active_jobs") {
            "该 Profile 仍有进行中的 Job；请先取消或等待任务结束。".to_string()
        } else {
            format!("无法删除 Embedding Profile：{error}")
        }
    })
}

#[tauri::command]
fn create_embedding_profile(
    app: AppHandle,
    database: State<'_, Database>,
    request: CreateEmbeddingProfileRequest,
) -> Result<EmbeddingProfile, String> {
    let provider = request.provider.trim().to_ascii_lowercase();
    let model = request.model.trim();
    let version = request.version.trim();
    if model.is_empty() || model.len() > 200 {
        return Err("Embedding 模型名称无效".to_string());
    }
    if version.is_empty() || version.len() > 100 {
        return Err("Embedding 模型版本无效".to_string());
    }
    if request.dimensions == 0 || request.dimensions > 65_536 {
        return Err("Embedding 维度无效".to_string());
    }
    embedding_profile_defaults(&provider)?;
    let keys = read_key_metadata(&app)?;
    validate_embedding_credential(&provider, &request.credential_id, &keys)?;
    read_credential_secret(&request.credential_id)?;
    let created_at =
        i64::try_from(unix_timestamp()?).map_err(|_| "Profile 时间超出支持范围".to_string())?;
    let profile = EmbeddingProfile {
        profile_id: Uuid::new_v4().to_string(),
        provider,
        model: model.to_string(),
        model_version: version.to_string(),
        dimensions: request.dimensions,
        input_schema_version: INPUT_SCHEMA_VERSION.to_string(),
        chunk_policy_version: CHUNK_POLICY_VERSION.to_string(),
        tokenizer: None,
        credential_id: Some(request.credential_id),
        status: ProfileStatus::Draft,
        is_active: false,
        created_at,
        activated_at: None,
        error: None,
    };
    database
        .upsert_profile(&profile)
        .map_err(|error| format!("无法创建 Embedding Profile：{error}"))?;
    ensure_adaptive_policy(database.inner(), &profile, created_at)?;
    Ok(profile)
}

#[tauri::command]
fn get_active_embedding_profile(
    database: State<'_, Database>,
) -> Result<Option<EmbeddingProfile>, String> {
    database
        .active_profile()
        .map_err(|error| format!("无法读取活动 Embedding Profile：{error}"))
}

#[tauri::command]
fn activate_ready_embedding_profile(
    app: AppHandle,
    database: State<'_, Database>,
    profile_id: String,
) -> Result<EmbeddingProfile, String> {
    let profile = database
        .profile(&profile_id)
        .map_err(|error| format!("无法读取 Embedding Profile：{error}"))?
        .ok_or_else(|| "未找到 Embedding Profile".to_string())?;
    if !matches!(profile.status, ProfileStatus::Ready | ProfileStatus::Active) {
        return Err("只有 Ready Profile 可以激活".to_string());
    }
    let credential_id = profile
        .credential_id
        .as_deref()
        .ok_or_else(|| "Embedding Profile 没有关联凭据".to_string())?;
    let keys = read_key_metadata(&app)?;
    validate_embedding_credential(&profile.provider, credential_id, &keys)?;
    read_credential_secret(credential_id)?;
    let activated_at =
        i64::try_from(unix_timestamp()?).map_err(|_| "Profile 时间超出支持范围".to_string())?;
    database
        .activate_profile_atomic(&profile_id, activated_at)
        .map_err(|error| format!("无法激活 Embedding Profile：{error}"))?;
    database
        .active_profile()
        .map_err(|error| format!("无法读取活动 Embedding Profile：{error}"))?
        .ok_or_else(|| "活动 Embedding Profile 状态丢失".to_string())
}

#[tauri::command]
fn list_embedding_adaptive_history(
    database: State<'_, Database>,
    profile_id: String,
) -> Result<Vec<AdaptiveHistoryRecord>, String> {
    database
        .adaptive_history(&profile_id)
        .map_err(|error| format!("无法读取 Profile 自适应历史：{error}"))
}

fn ensure_adaptive_policy(
    database: &Database,
    profile: &EmbeddingProfile,
    now: i64,
) -> Result<AdaptivePolicyState, String> {
    if let Some(policy) = database
        .adaptive_policy(&profile.profile_id)
        .map_err(|error| format!("无法读取自适应策略：{error}"))?
    {
        return Ok(policy);
    }
    let policy = initial_policy(profile, now);
    database
        .save_adaptive_policy(&policy)
        .map_err(|error| format!("无法初始化自适应策略：{error}"))?;
    Ok(policy)
}

fn create_adaptive_proposal(
    database: &Database,
    profile: &EmbeddingProfile,
    trigger: &str,
    now: i64,
) -> Result<AdaptiveHistoryRecord, String> {
    let current = ensure_adaptive_policy(database, profile, now)?;
    let signals = database
        .adaptive_signals(&profile.profile_id)
        .map_err(|error| format!("无法读取本地自适应信号：{error}"))?;
    let proposal = propose_policy(&current, &signals, now);
    proposal
        .policy
        .validate()
        .map_err(|error| format!("自适应提案无效：{error}"))?;
    let record = AdaptiveHistoryRecord {
        history_id: Uuid::new_v4().to_string(),
        profile_id: profile.profile_id.clone(),
        parameter_version: proposal.policy.policy_version.clone(),
        proposal: serde_json::to_value(&proposal).map_err(|error| error.to_string())?,
        evidence: serde_json::json!({
            "trigger": trigger,
            "beforePolicy": current,
            "signals": signals,
            "effectiveFor": "jobs_created_after_explicit_apply",
            "nonDestructive": true,
            "costIsOptimizationObjective": false,
        }),
        decision: "proposed".to_string(),
        created_at: now,
        applied_at: None,
    };
    database
        .append_adaptive_history(&record)
        .map_err(|error| format!("无法保存自适应提案：{error}"))?;
    Ok(record)
}

#[tauri::command]
fn set_local_adaptive_enabled(
    database: State<'_, Database>,
    profile_id: String,
    enabled: bool,
) -> Result<AdaptivePolicyState, String> {
    set_local_adaptive_enabled_impl(
        database.inner(),
        &profile_id,
        enabled,
        unix_timestamp_i64()?,
    )
}

fn set_local_adaptive_enabled_impl(
    database: &Database,
    profile_id: &str,
    enabled: bool,
    now: i64,
) -> Result<AdaptivePolicyState, String> {
    let profile = database
        .profile(profile_id)
        .map_err(|error| format!("无法读取 Profile：{error}"))?
        .ok_or_else(|| "Profile 不存在".to_string())?;
    let before = ensure_adaptive_policy(database, &profile, now)?;
    let mut updated = before.clone();
    updated.local_update_enabled = enabled;
    updated.policy_version = format!("{ADAPTIVE_POLICY_SCHEMA_VERSION}:toggle:{now}");
    updated.effective_after = now;
    updated.updated_at = now;
    database
        .save_adaptive_policy(&updated)
        .map_err(|error| format!("无法保存本地更新开关：{error}"))?;
    database
        .append_adaptive_history(&AdaptiveHistoryRecord {
            history_id: Uuid::new_v4().to_string(),
            profile_id: profile_id.to_string(),
            parameter_version: updated.policy_version.clone(),
            proposal: serde_json::json!({
                "kind": "local_update_toggle",
                "enabled": enabled,
                "policy": updated,
            }),
            evidence: serde_json::json!({
                "beforePolicy": before,
                "effectiveFor": "jobs_created_after_toggle",
            }),
            decision: "toggle_applied".to_string(),
            created_at: now,
            applied_at: Some(now),
        })
        .map_err(|error| format!("无法保存开关历史：{error}"))?;
    Ok(updated)
}

#[tauri::command]
fn propose_adaptive_settings(
    database: State<'_, Database>,
    profile_id: String,
) -> Result<AdaptiveHistoryRecord, String> {
    propose_adaptive_settings_impl(database.inner(), &profile_id, unix_timestamp_i64()?)
}

fn propose_adaptive_settings_impl(
    database: &Database,
    profile_id: &str,
    now: i64,
) -> Result<AdaptiveHistoryRecord, String> {
    let profile = database
        .profile(profile_id)
        .map_err(|error| format!("无法读取 Profile：{error}"))?
        .ok_or_else(|| "Profile 不存在".to_string())?;
    create_adaptive_proposal(database, &profile, "explicit_user_request", now)
}

fn parse_adaptive_proposal(record: &AdaptiveHistoryRecord) -> Result<AdaptiveProposal, String> {
    serde_json::from_value(record.proposal.clone())
        .map_err(|error| format!("自适应历史提案格式无效：{error}"))
}

#[tauri::command]
fn apply_adaptive_history(
    database: State<'_, Database>,
    history_id: String,
) -> Result<AdaptivePolicyState, String> {
    apply_adaptive_history_impl(database.inner(), &history_id, unix_timestamp_i64()?)
}

fn apply_adaptive_history_impl(
    database: &Database,
    history_id: &str,
    now: i64,
) -> Result<AdaptivePolicyState, String> {
    let record = database
        .adaptive_history_record(history_id)
        .map_err(|error| format!("无法读取自适应历史：{error}"))?
        .ok_or_else(|| "自适应历史不存在".to_string())?;
    if record.decision != "proposed" {
        return Err("只有 proposed 历史可以应用".to_string());
    }
    let proposal = parse_adaptive_proposal(&record)?;
    proposal.policy.validate()?;
    let profile = database
        .profile(&record.profile_id)
        .map_err(|error| format!("无法读取 Profile：{error}"))?
        .ok_or_else(|| "Profile 不存在".to_string())?;
    let maximum = initial_policy(&profile, record.created_at).h;
    if proposal.policy.h != maximum {
        return Err("提案 H 不符合当前 Provider/模型安全上限".to_string());
    }
    let current = ensure_adaptive_policy(database, &profile, now)?;
    if let Some(previous_id) = current.applied_history_id.as_deref() {
        database
            .update_adaptive_history_decision(
                previous_id,
                &["applied"],
                "superseded",
                Some(current.updated_at),
            )
            .map_err(|error| format!("无法更新旧自适应历史：{error}"))?;
    }
    let mut applied = proposal.policy;
    applied.local_update_enabled = current.local_update_enabled;
    applied.applied_history_id = Some(history_id.to_string());
    applied.effective_after = now;
    applied.updated_at = now;
    database
        .save_adaptive_policy(&applied)
        .map_err(|error| format!("无法应用自适应策略：{error}"))?;
    database
        .update_adaptive_history_decision(history_id, &["proposed"], "applied", Some(now))
        .map_err(|error| format!("无法更新自适应历史：{error}"))?;
    Ok(applied)
}

#[tauri::command]
fn reject_adaptive_history(
    database: State<'_, Database>,
    history_id: String,
) -> Result<AdaptiveHistoryRecord, String> {
    reject_adaptive_history_impl(database.inner(), &history_id)
}

fn reject_adaptive_history_impl(
    database: &Database,
    history_id: &str,
) -> Result<AdaptiveHistoryRecord, String> {
    database
        .update_adaptive_history_decision(history_id, &["proposed"], "rejected", None)
        .map_err(|error| format!("无法拒绝自适应历史：{error}"))?;
    database
        .adaptive_history_record(history_id)
        .map_err(|error| format!("无法读取自适应历史：{error}"))?
        .ok_or_else(|| "自适应历史不存在".to_string())
}

#[tauri::command]
fn revert_adaptive_history(
    database: State<'_, Database>,
    history_id: String,
) -> Result<AdaptivePolicyState, String> {
    revert_adaptive_history_impl(database.inner(), &history_id, unix_timestamp_i64()?)
}

fn revert_adaptive_history_impl(
    database: &Database,
    history_id: &str,
    now: i64,
) -> Result<AdaptivePolicyState, String> {
    let record = database
        .adaptive_history_record(history_id)
        .map_err(|error| format!("无法读取自适应历史：{error}"))?
        .ok_or_else(|| "自适应历史不存在".to_string())?;
    if record.decision != "applied" {
        return Err("只有当前 applied 历史可以回滚".to_string());
    }
    let current = database
        .adaptive_policy(&record.profile_id)
        .map_err(|error| format!("无法读取当前自适应策略：{error}"))?
        .ok_or_else(|| "当前自适应策略不存在".to_string())?;
    if current.applied_history_id.as_deref() != Some(history_id) {
        return Err("只能回滚当前生效的自适应历史".to_string());
    }
    let mut restored: AdaptivePolicyState = serde_json::from_value(
        record
            .evidence
            .get("beforePolicy")
            .cloned()
            .ok_or_else(|| "自适应历史缺少 beforePolicy".to_string())?,
    )
    .map_err(|error| format!("无法解析回滚策略：{error}"))?;
    restored.local_update_enabled = current.local_update_enabled;
    restored.effective_after = now;
    restored.updated_at = now;
    restored.validate()?;
    database
        .save_adaptive_policy(&restored)
        .map_err(|error| format!("无法回滚自适应策略：{error}"))?;
    database
        .update_adaptive_history_decision(history_id, &["applied"], "reverted", Some(now))
        .map_err(|error| format!("无法更新回滚历史：{error}"))?;
    if let Some(previous_id) = restored.applied_history_id.as_deref() {
        let _ = database.update_adaptive_history_decision(
            previous_id,
            &["superseded"],
            "applied",
            Some(now),
        );
    }
    Ok(restored)
}

#[tauri::command]
fn get_embedding_profile_settings(
    app: AppHandle,
    database: State<'_, Database>,
    profile_id: String,
) -> Result<EmbeddingProfileSettings, String> {
    let profile = database
        .profile(&profile_id)
        .map_err(|error| format!("无法读取 Embedding Profile：{error}"))?
        .ok_or_else(|| "未找到 Embedding Profile".to_string())?;
    let keys = read_key_metadata(&app)?;
    let credential_ready = profile
        .credential_id
        .as_deref()
        .is_some_and(|credential_id| {
            validate_embedding_credential(&profile.provider, credential_id, &keys).is_ok()
        });
    let defaults = embedding_profile_defaults(&profile.provider)?;
    let adaptive_history = database
        .adaptive_history(&profile_id)
        .map_err(|error| format!("无法读取 Profile 自适应历史：{error}"))?;
    let adaptive_policy =
        ensure_adaptive_policy(database.inner(), &profile, unix_timestamp_i64()?)?;
    Ok(EmbeddingProfileSettings {
        profile,
        defaults,
        credential_ready,
        adaptive_policy,
        adaptive_history,
    })
}

fn unix_timestamp_i64() -> Result<i64, String> {
    i64::try_from(unix_timestamp()?).map_err(|_| "时间超出支持范围".to_string())
}

fn current_snapshot_for_contract(
    app: &AppHandle,
    database: &Database,
) -> Result<CanonicalSnapshot, String> {
    load_or_refresh_canonical_snapshot(app, database)
}

fn preflight_profile_for_request(
    database: &Database,
    request: &ProfileChangeRequest,
) -> Result<EmbeddingProfile, String> {
    if let Some(profile_id) = request.target_profile_id.as_deref() {
        return database
            .profile(profile_id)
            .map_err(|error| format!("无法读取目标 Profile：{error}"))?
            .ok_or_else(|| "未找到目标 Embedding Profile".to_string());
    }
    let provider = request.provider.trim().to_ascii_lowercase();
    let defaults = embedding_profile_defaults(&provider)?;
    Ok(EmbeddingProfile {
        profile_id: "preflight-only".to_string(),
        provider,
        model: request.model.clone().unwrap_or(defaults.model),
        model_version: defaults.version,
        dimensions: defaults.dimensions,
        input_schema_version: INPUT_SCHEMA_VERSION.to_string(),
        chunk_policy_version: CHUNK_POLICY_VERSION.to_string(),
        tokenizer: None,
        credential_id: request.target_credential_id.clone(),
        status: ProfileStatus::Draft,
        is_active: false,
        created_at: unix_timestamp_i64()?,
        activated_at: None,
        error: None,
    })
}

fn emit_preflight(app: &AppHandle, estimate: &vectorization::PreflightEstimate) {
    let _ = app.emit("embedding-preflight-estimate", estimate);
}

#[tauri::command]
fn prepare_profile_change(
    app: AppHandle,
    database: State<'_, Database>,
    request: ProfileChangeRequest,
) -> Result<PreparedProfileChange, String> {
    let profile = preflight_profile_for_request(database.inner(), &request)?;
    let snapshot = current_snapshot_for_contract(&app, database.inner())?;
    let snapshot = snapshot_for_embedding(database.inner(), &snapshot)?;
    let estimate = estimate_preflight(&snapshot, &profile);
    let source = request
        .source_profile_id
        .as_deref()
        .map(|profile_id| database.profile(profile_id))
        .transpose()
        .map_err(|error| format!("无法读取源 Profile：{error}"))?
        .flatten();
    let compatible = source.as_ref().is_some_and(|source| {
        source.provider == profile.provider
            && source.model == profile.model
            && source.model_version == profile.model_version
            && source.dimensions == profile.dimensions
            && source.input_schema_version == profile.input_schema_version
            && source.chunk_policy_version == profile.chunk_policy_version
    });
    let reason = (!compatible).then(|| "profile_vector_space_changed".to_string());
    emit_preflight(&app, &estimate);
    Ok(PreparedProfileChange {
        compatible,
        reason,
        request,
        estimate: Some(estimate),
    })
}

fn profile_change_preflight(
    app: &AppHandle,
    database: &Database,
    request: &ProfileChangeRequest,
) -> Result<vectorization::PreflightEstimate, String> {
    let profile = preflight_profile_for_request(database, request)?;
    let snapshot = current_snapshot_for_contract(app, database)?;
    let snapshot = snapshot_for_embedding(database, &snapshot)?;
    let estimate = estimate_preflight(&snapshot, &profile);
    emit_preflight(app, &estimate);
    Ok(estimate)
}

#[tauri::command]
fn begin_profile_rebuild_preflight(
    app: AppHandle,
    database: State<'_, Database>,
    request: ProfileChangeRequest,
) -> Result<vectorization::PreflightEstimate, String> {
    profile_change_preflight(&app, database.inner(), &request)
}

#[tauri::command]
fn begin_profile_migration_preflight(
    app: AppHandle,
    database: State<'_, Database>,
    request: ProfileChangeRequest,
) -> Result<vectorization::PreflightEstimate, String> {
    profile_change_preflight(&app, database.inner(), &request)
}

#[tauri::command]
fn begin_full_rebuild_preflight(
    app: AppHandle,
    database: State<'_, Database>,
    profile_id: Option<String>,
) -> Result<vectorization::PreflightEstimate, String> {
    let profile = if let Some(profile_id) = profile_id {
        database
            .profile(&profile_id)
            .map_err(|error| format!("无法读取 Profile：{error}"))?
            .ok_or_else(|| "未找到 Embedding Profile".to_string())?
    } else {
        database
            .active_profile()
            .map_err(|error| format!("无法读取活动 Profile：{error}"))?
            .ok_or_else(|| "当前没有活动 Embedding Profile".to_string())?
    };
    let snapshot = current_snapshot_for_contract(&app, database.inner())?;
    let snapshot = snapshot_for_embedding(database.inner(), &snapshot)?;
    let estimate = estimate_preflight(&snapshot, &profile);
    emit_preflight(&app, &estimate);
    Ok(estimate)
}

fn diff_for_active_profile(
    database: &Database,
    snapshot: &CanonicalSnapshot,
) -> Result<(Option<EmbeddingProfile>, IndexDiff), String> {
    let active = database
        .active_profile()
        .map_err(|error| format!("无法读取活动 Profile：{error}"))?;
    let diff = if let Some(profile) = &active {
        database
            .compute_index_diff(&profile.profile_id, snapshot)
            .map_err(|error| format!("无法计算索引差异：{error}"))?
    } else {
        IndexDiff {
            added: snapshot
                .skills
                .iter()
                .filter(|skill| !skill.path.starts_with("bundled://"))
                .count() as u64,
            changed: 0,
            removed: 0,
            unchanged: 0,
        }
    };
    Ok((active, diff))
}

fn snapshot_for_embedding(
    database: &Database,
    snapshot: &CanonicalSnapshot,
) -> Result<CanonicalSnapshot, String> {
    let mut filtered = snapshot.clone();
    if database
        .ignore_built_in_skills()
        .map_err(|error| format!("无法读取内置 Skills 设置：{error}"))?
    {
        filtered.skills.retain(|skill| !skill.is_built_in);
    }
    Ok(filtered)
}

#[tauri::command]
fn scan_embedding_changes(
    app: AppHandle,
    database: State<'_, Database>,
) -> Result<IndexDiff, String> {
    let snapshot = build_canonical_snapshot(&app, database.inner())?;
    database
        .replace_canonical_snapshot(&snapshot)
        .map_err(|error| format!("无法保存变更扫描快照：{error}"))?;
    let embedding_snapshot = snapshot_for_embedding(database.inner(), &snapshot)?;
    let (_, diff) = diff_for_active_profile(database.inner(), &embedding_snapshot)?;
    database
        .save_index_diff(&diff)
        .map_err(|error| format!("无法保存索引差异：{error}"))?;
    Ok(diff)
}

fn incremental_work_snapshot(
    database: &Database,
    profile_id: &str,
    snapshot: &CanonicalSnapshot,
) -> Result<CanonicalSnapshot, String> {
    let unchanged = database
        .unchanged_indexed_skill_ids(profile_id, snapshot)
        .map_err(|error| format!("无法读取已复用 Skills：{error}"))?;
    let mut work = snapshot.clone();
    work.skills.retain(|skill| {
        !skill.path.starts_with("bundled://") && !unchanged.contains(&skill.skill_id)
    });
    Ok(work)
}

#[tauri::command]
fn get_index_diff(database: State<'_, Database>) -> Result<IndexDiff, String> {
    database
        .last_index_diff()
        .map_err(|error| format!("无法读取索引差异：{error}"))
}

#[tauri::command]
fn get_index_sync_status(
    app: AppHandle,
    database: State<'_, Database>,
) -> Result<IndexSyncStatus, String> {
    let snapshot = current_snapshot_for_contract(&app, database.inner())?;
    let snapshot = snapshot_for_embedding(database.inner(), &snapshot)?;
    let (active, diff) = diff_for_active_profile(database.inner(), &snapshot)?;
    database
        .index_sync_status(
            active.as_ref().map(|profile| profile.profile_id.as_str()),
            diff.added + diff.changed + diff.removed,
        )
        .map_err(|error| format!("无法读取索引同步状态：{error}"))
}

#[tauri::command]
fn set_index_auto_update(
    app: AppHandle,
    database: State<'_, Database>,
    enabled: bool,
) -> Result<IndexSyncStatus, String> {
    database
        .set_index_auto_update(enabled)
        .map_err(|error| format!("无法保存自动更新设置：{error}"))?;
    get_index_sync_status(app, database)
}

#[tauri::command]
fn set_ignore_built_in_skills(
    app: AppHandle,
    database: State<'_, Database>,
    enabled: bool,
) -> Result<IndexSyncStatus, String> {
    database
        .set_ignore_built_in_skills(enabled)
        .map_err(|error| format!("无法保存内置 Skills 设置：{error}"))?;
    get_index_sync_status(app, database)
}

#[tauri::command]
fn list_embedding_jobs(database: State<'_, Database>) -> Result<Vec<EmbeddingJob>, String> {
    database
        .list_jobs()
        .map_err(|error| format!("无法读取 Embedding Jobs：{error}"))
}

#[tauri::command]
fn delete_embedding_job_history(
    database: State<'_, Database>,
    job_id: String,
) -> Result<bool, String> {
    database
        .delete_job_history(Some(&job_id))
        .map(|deleted| deleted > 0)
        .map_err(|error| format!("无法删除 Embedding Job 历史：{error}"))
}

#[tauri::command]
fn clear_embedding_job_history(database: State<'_, Database>) -> Result<u64, String> {
    database
        .delete_job_history(None)
        .map_err(|error| format!("无法清空 Embedding Jobs 历史：{error}"))
}

struct StagedJobTarget {
    profile: EmbeddingProfile,
    source_profile_id: Option<String>,
    staged_profile: bool,
    key_rotation_only: bool,
}

fn clone_profile_for_rebuild(
    source: &EmbeddingProfile,
    credential_id: String,
    created_at: i64,
) -> EmbeddingProfile {
    EmbeddingProfile {
        profile_id: Uuid::new_v4().to_string(),
        provider: source.provider.clone(),
        model: source.model.clone(),
        model_version: source.model_version.clone(),
        dimensions: source.dimensions,
        input_schema_version: source.input_schema_version.clone(),
        chunk_policy_version: source.chunk_policy_version.clone(),
        tokenizer: source.tokenizer.clone(),
        credential_id: Some(credential_id),
        status: ProfileStatus::Draft,
        is_active: false,
        created_at,
        activated_at: None,
        error: None,
    }
}

fn stage_embedding_job_target(
    database: &Database,
    keys: &[ApiKeyMetadata],
    strategy: JobStrategy,
    request: Option<&ProfileChangeRequest>,
    profile_id: Option<&str>,
    created_at: i64,
) -> Result<StagedJobTarget, String> {
    let active = database
        .active_profile()
        .map_err(|error| format!("无法读取活动 Profile：{error}"))?;
    let requested_profile_id = profile_id
        .map(str::to_string)
        .or_else(|| request.and_then(|request| request.target_profile_id.clone()));

    if strategy == JobStrategy::Incremental {
        let target = if let Some(profile_id) = requested_profile_id {
            database
                .profile(&profile_id)
                .map_err(|error| format!("无法读取目标 Profile：{error}"))?
                .ok_or_else(|| "未找到目标 Embedding Profile".to_string())?
        } else {
            active
                .clone()
                .ok_or_else(|| "增量更新需要活动 Embedding Profile".to_string())?
        };
        if !target.is_active {
            return Err("增量更新只能写入活动 Embedding Profile".to_string());
        }
        let credential_id = target
            .credential_id
            .as_deref()
            .ok_or_else(|| "活动 Profile 没有 Embedding 凭据".to_string())?;
        validate_embedding_credential_by_id(&target.provider, credential_id, keys, true)?;
        return Ok(StagedJobTarget {
            source_profile_id: Some(target.profile_id.clone()),
            profile: target,
            staged_profile: false,
            key_rotation_only: false,
        });
    }

    if let Some(profile_id) = requested_profile_id {
        let target = database
            .profile(&profile_id)
            .map_err(|error| format!("无法读取目标 Profile：{error}"))?
            .ok_or_else(|| "未找到目标 Embedding Profile".to_string())?;
        let credential_id = request
            .and_then(|request| request.target_credential_id.clone())
            .or_else(|| target.credential_id.clone())
            .ok_or_else(|| "目标 Profile 没有 Embedding 凭据".to_string())?;
        validate_embedding_credential_by_id(&target.provider, &credential_id, keys, false)?;
        if target.is_active
            && request.is_some_and(|request| {
                request.reason == "credential"
                    && request.provider == target.provider
                    && request
                        .model
                        .as_ref()
                        .is_none_or(|model| model == &target.model)
            })
        {
            let mut target = target;
            target.credential_id = Some(credential_id);
            return Ok(StagedJobTarget {
                source_profile_id: Some(target.profile_id.clone()),
                profile: target,
                staged_profile: false,
                key_rotation_only: true,
            });
        }
        if target.is_active {
            let clone = clone_profile_for_rebuild(&target, credential_id, created_at);
            database
                .upsert_profile(&clone)
                .map_err(|error| format!("无法创建隔离重建 Profile：{error}"))?;
            return Ok(StagedJobTarget {
                source_profile_id: Some(target.profile_id),
                profile: clone,
                staged_profile: true,
                key_rotation_only: false,
            });
        }
        let mut target = target;
        if target.credential_id.as_deref() != Some(credential_id.as_str()) {
            target.credential_id = Some(credential_id);
            database
                .upsert_profile(&target)
                .map_err(|error| format!("无法更新目标 Profile：{error}"))?;
        }
        return Ok(StagedJobTarget {
            source_profile_id: active.map(|profile| profile.profile_id),
            profile: target,
            staged_profile: false,
            key_rotation_only: false,
        });
    }

    let request = request.ok_or_else(|| "缺少 Profile 变更请求".to_string())?;
    let credential_id = request
        .target_credential_id
        .clone()
        .ok_or_else(|| "Profile 变更请求缺少 targetCredentialId".to_string())?;
    let provider = request.provider.trim().to_ascii_lowercase();
    validate_embedding_credential_by_id(&provider, &credential_id, keys, false)?;
    let defaults = embedding_profile_defaults(&provider)?;
    let model = request.model.clone().unwrap_or(defaults.model.clone());
    if request.reason == "credential"
        && active.as_ref().is_some_and(|profile| {
            profile.provider == provider
                && request
                    .model
                    .as_ref()
                    .is_none_or(|requested_model| requested_model == &profile.model)
        })
    {
        let mut profile = active.expect("active profile checked above");
        profile.credential_id = Some(credential_id);
        return Ok(StagedJobTarget {
            source_profile_id: Some(profile.profile_id.clone()),
            profile,
            staged_profile: false,
            key_rotation_only: true,
        });
    }
    let profile = EmbeddingProfile {
        profile_id: Uuid::new_v4().to_string(),
        provider,
        model,
        model_version: defaults.version,
        dimensions: defaults.dimensions,
        input_schema_version: INPUT_SCHEMA_VERSION.to_string(),
        chunk_policy_version: CHUNK_POLICY_VERSION.to_string(),
        tokenizer: None,
        credential_id: Some(credential_id),
        status: ProfileStatus::Draft,
        is_active: false,
        created_at,
        activated_at: None,
        error: None,
    };
    database
        .upsert_profile(&profile)
        .map_err(|error| format!("无法暂存目标 Profile：{error}"))?;
    Ok(StagedJobTarget {
        source_profile_id: active.map(|profile| profile.profile_id),
        profile,
        staged_profile: true,
        key_rotation_only: false,
    })
}

fn update_embedding_role_metadata(
    app: &AppHandle,
    database_update: impl FnOnce() -> Result<(), String>,
    credential_id: &str,
) -> Result<(), String> {
    let original = read_key_metadata(app)?;
    let mut updated = original.clone();
    activate_metadata_for_purpose(&mut updated, credential_id, CredentialPurpose::Embedding)?;
    write_key_metadata(app, &updated)?;
    if let Err(error) = database_update() {
        let _ = write_key_metadata(app, &original);
        return Err(error);
    }
    Ok(())
}

fn rotate_same_profile_credential(
    app: &AppHandle,
    database: &Database,
    profile: &EmbeddingProfile,
    credential_id: &str,
    updated_at: i64,
) -> Result<(), String> {
    update_embedding_role_metadata(
        app,
        || {
            database
                .rotate_active_profile_credential_atomic(
                    &profile.profile_id,
                    credential_id,
                    &profile.provider,
                    updated_at,
                )
                .map_err(|error| format!("无法切换 Embedding 凭据：{error}"))
        },
        credential_id,
    )
}

#[tauri::command]
fn start_embedding_job(
    app: AppHandle,
    database: State<'_, Database>,
    strategy: JobStrategy,
    request: Option<ProfileChangeRequest>,
    profile_id: Option<String>,
) -> Result<EmbeddingJob, String> {
    let now = unix_timestamp_i64()?;
    let keys = read_key_metadata(&app)?;
    let staged = stage_embedding_job_target(
        database.inner(),
        &keys,
        strategy.clone(),
        request.as_ref(),
        profile_id.as_deref(),
        now,
    )?;
    let credential_id = staged
        .profile
        .credential_id
        .clone()
        .ok_or_else(|| "目标 Profile 没有 Embedding 凭据".to_string())?;
    read_credential_secret(&credential_id)?;
    if staged.key_rotation_only {
        rotate_same_profile_credential(
            &app,
            database.inner(),
            &staged.profile,
            &credential_id,
            now,
        )?;
        let job = EmbeddingJob {
            job_id: Uuid::new_v4().to_string(),
            profile_id: staged.profile.profile_id,
            kind: strategy.as_job_kind(),
            status: JobStatus::Completed,
            total_items: 0,
            completed_items: 0,
            estimated_tokens: 0,
            actual_tokens: 0,
            estimated_cost_low: None,
            estimated_cost_high: None,
            created_at: now,
            updated_at: now,
            error: None,
        };
        database
            .create_job(&job)
            .map_err(|error| format!("无法记录凭据轮换 Job：{error}"))?;
        return Ok(job);
    }
    let profile = staged.profile;
    let snapshot = current_snapshot_for_contract(&app, database.inner())?;
    let snapshot = snapshot_for_embedding(database.inner(), &snapshot)?;
    let estimate_snapshot = if strategy == JobStrategy::Incremental {
        incremental_work_snapshot(database.inner(), &profile.profile_id, &snapshot)?
    } else {
        snapshot.clone()
    };
    let estimate = estimate_preflight(&estimate_snapshot, &profile);
    let job = EmbeddingJob {
        job_id: Uuid::new_v4().to_string(),
        profile_id: profile.profile_id.clone(),
        kind: strategy.as_job_kind(),
        status: JobStatus::Pending,
        total_items: estimate.parent_count + estimate.chunk_count_high,
        completed_items: 0,
        estimated_tokens: estimate.embedding_tokens_high,
        actual_tokens: 0,
        estimated_cost_low: estimate.estimated_cost_low,
        estimated_cost_high: estimate.estimated_cost_high,
        created_at: now,
        updated_at: now,
        error: None,
    };
    database
        .create_job(&job)
        .map_err(|error| format!("无法创建 Embedding Job：{error}"))?;
    let adaptive_policy = ensure_adaptive_policy(database.inner(), &profile, now)?;
    let context = EmbeddingJobContext {
        source_profile_id: staged.source_profile_id,
        target_credential_id: credential_id,
        activate_on_success: strategy != JobStrategy::Incremental,
        staged_profile: staged.staged_profile,
        adaptive_policy,
    };
    database
        .save_job_context(
            &job.job_id,
            &serde_json::to_value(context).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("无法保存 Job 上下文：{error}"))?;
    let app_for_job = app.clone();
    let job_for_task = job.clone();
    tauri::async_runtime::spawn(async move {
        run_embedding_job(app_for_job, job_for_task).await;
    });
    Ok(job)
}

#[tauri::command]
fn cancel_embedding_job(database: State<'_, Database>, job_id: String) -> Result<bool, String> {
    database
        .request_job_cancel(&job_id)
        .map_err(|error| format!("无法取消 Embedding Job：{error}"))
}

#[tauri::command]
async fn semantic_search(
    app: AppHandle,
    database: State<'_, Database>,
    query: String,
    agent_filter: Option<String>,
    vector_types: Option<Vec<VectorType>>,
    include_disabled_skills: Option<bool>,
) -> Result<Vec<SearchResult>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let profile = database
        .active_profile()
        .map_err(|error| format!("无法读取活动 Profile：{error}"))?
        .ok_or_else(|| "语义搜索需要活动 Embedding Profile".to_string())?;
    let credential_id = profile
        .credential_id
        .as_deref()
        .ok_or_else(|| "活动 Profile 没有 Embedding 凭据".to_string())?;
    let keys = read_key_metadata(&app)?;
    validate_embedding_credential(&profile.provider, credential_id, &keys)?;
    let api_key = read_credential_secret(credential_id)?;
    let selected_types = vector_types.unwrap_or_else(|| VectorType::ALL.to_vec());
    if selected_types.is_empty() {
        return Ok(Vec::new());
    }
    let texts = selected_types
        .iter()
        .map(|vector_type| format!("type: {vector_type}\nquery: {query}"))
        .collect::<Vec<_>>();
    let provider = RemoteEmbeddingProvider::new(&profile, api_key, ReqwestJsonTransport);
    let batch = provider
        .embed(&texts)
        .await
        .map_err(|error| format!("无法嵌入搜索查询：{error}"))?;
    let query_vectors = QueryVectors {
        profile_id: profile.profile_id.clone(),
        vectors: selected_types
            .into_iter()
            .zip(batch.vectors)
            .collect::<BTreeMap<_, _>>(),
    };
    let snapshot = database
        .current_canonical_snapshot()
        .map_err(|error| format!("无法读取规范快照：{error}"))?;
    let include_disabled_skills = include_disabled_skills.unwrap_or(
        database
            .include_disabled_search()
            .map_err(|error| format!("无法读取搜索设置：{error}"))?,
    );
    let allowed = snapshot
        .as_ref()
        .map(|snapshot| snapshot_for_embedding(database.inner(), snapshot))
        .transpose()?
        .map(|snapshot| {
            snapshot
                .skills
                .into_iter()
                .filter(|skill| {
                    agent_filter.as_ref().is_none_or(|agent| {
                        skill.enabled_agents.iter().any(|enabled| enabled == agent)
                            && (include_disabled_skills
                                || !skill
                                    .disabled_agents
                                    .iter()
                                    .any(|disabled| disabled == agent))
                    })
                })
                .map(|skill| skill.skill_id)
                .collect::<HashSet<_>>()
        });
    let vectors = database
        .load_vectors(&profile.profile_id)
        .map_err(|error| format!("无法读取活动 Profile 向量：{error}"))?;
    retrieve_semantic_results(
        &query_vectors,
        &vectors,
        allowed.as_ref(),
        &RetrievalConfig::default(),
    )
    .map_err(|error| format!("语义检索失败：{error}"))
}

#[tauri::command]
fn list_skill_relations(
    database: State<'_, Database>,
    skill_id: String,
    profile_id: Option<String>,
) -> Result<Vec<SkillRelationship>, String> {
    database
        .relationships_for_skill_profile(&skill_id, profile_id.as_deref())
        .map_err(|error| format!("无法读取 Skill 关系：{error}"))
}

#[tauri::command]
fn get_skill_graph(
    database: State<'_, Database>,
    view_id: String,
) -> Result<SkillGraphSnapshot, String> {
    if !matches!(view_id.as_str(), "all" | "claude-code" | "cursor" | "codex") {
        return Err(format!("未知 Skills 分类：{view_id}"));
    }
    let Some(profile) = database
        .active_profile()
        .map_err(|error| format!("无法读取活动 Profile：{error}"))?
    else {
        return Ok(empty_graph(&view_id));
    };
    let Some(snapshot) = database
        .current_canonical_snapshot()
        .map_err(|error| format!("无法读取规范快照：{error}"))?
    else {
        let mut graph = empty_graph(&view_id);
        graph.profile_id = Some(profile.profile_id);
        graph.graph_version = format!(
            "no-canonical-snapshot:{}",
            graph.profile_id.as_deref().unwrap()
        );
        graph.layout_version = graph.graph_version.clone();
        return Ok(graph);
    };
    let snapshot = snapshot_for_embedding(database.inner(), &snapshot)?;
    let vectors = database
        .load_vectors(&profile.profile_id)
        .map_err(|error| format!("无法读取活动 Profile 向量：{error}"))?;
    let relationships = database
        .relationships_for_profile(&profile.profile_id)
        .map_err(|error| format!("无法读取活动 Profile 关系：{error}"))?;
    Ok(build_skill_graph(
        &snapshot,
        &profile.profile_id,
        &vectors,
        &relationships,
        &view_id,
    ))
}

#[tauri::command]
fn generate_local_validation_samples(
    app: AppHandle,
    database: State<'_, Database>,
    profile_id: Option<String>,
) -> Result<Vec<ValidationSampleRecord>, String> {
    if let Some(profile_id) = profile_id.as_deref() {
        database
            .profile(profile_id)
            .map_err(|error| format!("无法读取验证 Profile：{error}"))?
            .ok_or_else(|| "验证 Profile 不存在".to_string())?;
    }
    let snapshot = current_snapshot_for_contract(&app, database.inner())?;
    let analyses = database
        .saved_analysis_records()
        .map_err(|error| format!("无法读取本地分析结果：{error}"))?;
    let samples = generate_samples(
        &snapshot,
        &analyses,
        profile_id.as_deref(),
        unix_timestamp_i64()?,
    );
    for sample in &samples {
        database
            .save_validation_sample(sample)
            .map_err(|error| format!("无法保存本地验证样本：{error}"))?;
    }
    Ok(samples)
}

#[tauri::command]
fn list_local_validation_samples(
    database: State<'_, Database>,
    profile_id: Option<String>,
    split: Option<String>,
) -> Result<Vec<ValidationSampleRecord>, String> {
    if split
        .as_deref()
        .is_some_and(|split| !matches!(split, "tuning" | "validation" | "holdout"))
    {
        return Err("split 必须是 tuning、validation 或 holdout".to_string());
    }
    database
        .list_validation_samples(profile_id.as_deref(), split.as_deref())
        .map_err(|error| format!("无法读取本地验证样本：{error}"))
}

#[tauri::command]
fn list_local_validation_runs(
    database: State<'_, Database>,
    profile_id: Option<String>,
) -> Result<Vec<ValidationRunRecord>, String> {
    database
        .list_validation_runs(profile_id.as_deref())
        .map_err(|error| format!("无法读取本地验证运行：{error}"))
}

#[tauri::command]
fn export_local_validation_data(
    database: State<'_, Database>,
    profile_id: Option<String>,
) -> Result<serde_json::Value, String> {
    build_local_validation_export(
        database.inner(),
        profile_id.as_deref(),
        unix_timestamp_i64()?,
    )
}

fn build_local_validation_export(
    database: &Database,
    profile_id: Option<&str>,
    exported_at: i64,
) -> Result<serde_json::Value, String> {
    let samples = database
        .list_validation_samples(profile_id, None)
        .map_err(|error| format!("无法导出本地验证样本：{error}"))?;
    let runs = database
        .list_validation_runs(profile_id)
        .map_err(|error| format!("无法导出本地验证运行：{error}"))?;
    let feedback = database
        .list_feedback_events(profile_id)
        .map_err(|error| format!("无法导出本地反馈：{error}"))?;
    Ok(serde_json::json!({
        "schemaVersion": LOCAL_DATASET_SCHEMA_VERSION,
        "localOnly": true,
        "externalWritePerformed": false,
        "profileId": profile_id,
        "exportedAt": exported_at,
        "samples": samples,
        "runs": runs,
        "feedbackEvents": feedback,
    }))
}

#[tauri::command]
fn reset_local_validation_data(
    database: State<'_, Database>,
    profile_id: Option<String>,
) -> Result<LocalValidationMutation, String> {
    database
        .reset_local_validation_data(profile_id.as_deref())
        .map_err(|error| format!("无法重置本地验证数据：{error}"))
}

#[tauri::command]
fn delete_local_validation_sample(
    database: State<'_, Database>,
    sample_id: String,
) -> Result<bool, String> {
    database
        .delete_validation_sample(&sample_id)
        .map_err(|error| format!("无法删除本地验证样本：{error}"))
}

fn pending_validation_run(
    profile_id: String,
    dataset_id: String,
    dataset_content_version: String,
    started_at: i64,
) -> ValidationRunRecord {
    ValidationRunRecord {
        run_id: Uuid::new_v4().to_string(),
        dataset_id,
        dataset_content_version,
        profile_id,
        status: "pending".to_string(),
        metrics: serde_json::json!({
            "advisoryOnly": true,
            "blocksActivation": false,
            "warnings": ["validation_running_in_background"]
        }),
        started_at,
        completed_at: None,
        error: None,
    }
}

fn effective_validation_dataset(
    samples: &[ValidationSampleRecord],
) -> (String, String, Vec<ValidationSampleRecord>) {
    let base_version = samples
        .iter()
        .find(|sample| {
            !matches!(
                sample.state,
                ValidationState::HumanConfirmed
                    | ValidationState::HumanRejected
                    | ValidationState::HumanReverted
            )
        })
        .map(|sample| sample.dataset_content_version.as_str());
    let reverted_feedback = samples
        .iter()
        .filter(|sample| sample.state == ValidationState::HumanReverted)
        .filter_map(|sample| {
            sample
                .evidence
                .get("feedbackId")
                .and_then(serde_json::Value::as_str)
        })
        .collect::<HashSet<_>>();
    let mut effective = samples
        .iter()
        .filter(|sample| {
            if matches!(
                sample.state,
                ValidationState::HumanConfirmed | ValidationState::HumanRejected
            ) {
                return sample
                    .evidence
                    .get("feedbackId")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|feedback_id| !reverted_feedback.contains(feedback_id));
            }
            sample.state != ValidationState::HumanReverted
                && base_version.is_none_or(|version| sample.dataset_content_version == version)
        })
        .cloned()
        .collect::<Vec<_>>();
    effective.sort_by(|left, right| left.sample_id.cmp(&right.sample_id));
    let dataset_id = effective
        .first()
        .map(|sample| sample.dataset_id.clone())
        .unwrap_or_else(|| "user-local:empty".to_string());
    let version_material = effective
        .iter()
        .map(|sample| format!("{}\0{}", sample.sample_id, sample.dataset_content_version))
        .collect::<Vec<_>>()
        .join("\n");
    let version = stable_hash(version_material.as_bytes());
    (dataset_id, version, effective)
}

#[tauri::command]
fn run_local_validation(
    app: AppHandle,
    database: State<'_, Database>,
    profile_id: String,
) -> Result<ValidationRunRecord, String> {
    database
        .profile(&profile_id)
        .map_err(|error| format!("无法读取验证 Profile：{error}"))?
        .ok_or_else(|| "验证 Profile 不存在".to_string())?;
    let mut samples = database
        .list_validation_samples(Some(&profile_id), None)
        .map_err(|error| format!("无法读取本地验证样本：{error}"))?;
    if samples.is_empty() {
        samples = generate_local_validation_samples(
            app.clone(),
            database.clone(),
            Some(profile_id.clone()),
        )?;
    }
    let (dataset_id, dataset_content_version, effective_samples) =
        effective_validation_dataset(&samples);
    let started_at = unix_timestamp_i64()?;
    let run = pending_validation_run(profile_id, dataset_id, dataset_content_version, started_at);
    database
        .save_validation_run(&run)
        .map_err(|error| format!("无法创建本地验证运行：{error}"))?;
    let app_for_run = app.clone();
    let run_for_task = run.clone();
    tauri::async_runtime::spawn(async move {
        run_local_validation_worker(app_for_run, run_for_task, effective_samples).await;
    });
    Ok(run)
}

async fn run_local_validation_worker(
    app: AppHandle,
    run: ValidationRunRecord,
    samples: Vec<ValidationSampleRecord>,
) {
    let database = app.state::<Database>();
    if let Err(error) =
        run_local_validation_worker_inner(database.inner(), &app, &run, samples).await
    {
        let failed = ValidationRunRecord {
            status: "failed".to_string(),
            metrics: serde_json::json!({
                "advisoryOnly": true,
                "blocksActivation": false,
                "confidence": "low",
                "warnings": ["validation_worker_failed"]
            }),
            completed_at: Some(unix_timestamp_i64().unwrap_or_default()),
            error: Some(error),
            ..run
        };
        let _ = database.save_validation_run(&failed);
    }
}

async fn run_local_validation_worker_inner(
    database: &Database,
    app: &AppHandle,
    run: &ValidationRunRecord,
    samples: Vec<ValidationSampleRecord>,
) -> Result<(), String> {
    let samples = samples
        .into_iter()
        .filter(|sample| sample.split != "tuning")
        .collect::<Vec<_>>();
    if samples.is_empty() {
        let completed = ValidationRunRecord {
            status: "completed".to_string(),
            metrics: serde_json::to_value(compute_metrics(&[], 5))
                .map_err(|error| error.to_string())?,
            completed_at: Some(unix_timestamp_i64()?),
            error: None,
            ..run.clone()
        };
        return database
            .save_validation_run(&completed)
            .map_err(|error| error.to_string());
    }
    let profile = database
        .profile(&run.profile_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "验证 Profile 不存在".to_string())?;
    let credential_id = profile
        .credential_id
        .as_deref()
        .ok_or_else(|| "验证 Profile 没有 Embedding 凭据".to_string())?;
    let keys = read_key_metadata(app)?;
    validate_embedding_credential_by_id(&profile.provider, credential_id, &keys, false)?;
    let api_key = read_credential_secret(credential_id)?;
    let vectors = database
        .load_vectors(&profile.profile_id)
        .map_err(|error| error.to_string())?;
    let provider = RemoteEmbeddingProvider::new(&profile, api_key, ReqwestJsonTransport);
    let mut request_texts = Vec::new();
    let mut request_layout = Vec::new();
    for sample in &samples {
        let types = validation_vector_types(&sample.task_type);
        let start = request_texts.len();
        request_texts.extend(
            types
                .iter()
                .map(|vector_type| format!("type: {vector_type}\nquery: {}", sample.query_text)),
        );
        request_layout.push((start, types));
    }
    let embedded = provider
        .embed(&request_texts)
        .await
        .map_err(|error| error.to_string())?;
    let mut evaluated = Vec::new();
    for (sample, (start, types)) in samples.into_iter().zip(request_layout) {
        let query_vectors = QueryVectors {
            profile_id: profile.profile_id.clone(),
            vectors: types
                .into_iter()
                .enumerate()
                .map(|(offset, vector_type)| {
                    (vector_type, embedded.vectors[start + offset].clone())
                })
                .collect(),
        };
        let results =
            retrieve_semantic_results(&query_vectors, &vectors, None, &RetrievalConfig::default())
                .map_err(|error| error.to_string())?;
        evaluated.push(EvaluatedSample { sample, results });
    }
    let metrics = compute_metrics(&evaluated, 5);
    let completed = ValidationRunRecord {
        status: "completed".to_string(),
        metrics: serde_json::to_value(metrics).map_err(|error| error.to_string())?,
        completed_at: Some(unix_timestamp_i64()?),
        error: None,
        ..run.clone()
    };
    database
        .save_validation_run(&completed)
        .map_err(|error| error.to_string())
}

fn validation_vector_types(task_type: &str) -> Vec<VectorType> {
    let single = match task_type {
        "overall_function" => Some(VectorType::OverallFunction),
        "trigger" | "forbidden" => Some(VectorType::Trigger),
        "workflow" => Some(VectorType::Workflow),
        "resource" => Some(VectorType::Resource),
        _ => None,
    };
    single.map_or_else(|| VectorType::ALL.to_vec(), |vector_type| vec![vector_type])
}

#[tauri::command]
fn record_feedback_event(
    database: State<'_, Database>,
    request: FeedbackRequest,
) -> Result<FeedbackEventRecord, String> {
    request.validate()?;
    if let Some(profile_id) = request.profile_id.as_deref() {
        database
            .profile(profile_id)
            .map_err(|error| format!("无法读取反馈 Profile：{error}"))?
            .ok_or_else(|| "反馈 Profile 不存在".to_string())?;
    }
    let now = unix_timestamp_i64()?;
    let feedback_id = Uuid::new_v4().to_string();
    let dataset_id = request
        .dataset_id
        .clone()
        .unwrap_or_else(|| "user-local:feedback".to_string());
    let after_state = serde_json::json!({
        "action": request.action.as_str(),
        "skillId": request.skill_id,
        "otherSkillId": request.other_skill_id,
        "queryId": request.query_id,
        "queryText": request.query_text,
        "relationType": request.relation_type,
        "strength": request.strength,
    });
    let event = FeedbackEventRecord {
        feedback_id: feedback_id.clone(),
        dataset_id: dataset_id.clone(),
        skill_id: request.skill_id.clone(),
        query_id: request.query_id.clone(),
        other_skill_id: request.other_skill_id.clone(),
        relation_type: request
            .relation_type
            .map(RelationshipType::as_str)
            .map(str::to_string),
        action: request.action.as_str().to_string(),
        strength: request.strength,
        before_state: serde_json::json!({}),
        after_state,
        profile_id: request.profile_id.clone(),
        source_evidence: request.source_evidence.clone(),
        confirmed_at: now,
        reverted_at: None,
    };
    let relationship = feedback_relationship(&feedback_id, &request, now);
    let sample = feedback_sample(&feedback_id, &dataset_id, &request, now);
    database
        .record_feedback_bundle(&event, relationship.as_ref(), sample.as_ref())
        .map_err(|error| format!("无法保存语义反馈：{error}"))?;
    Ok(event)
}

fn feedback_relationship(
    feedback_id: &str,
    request: &FeedbackRequest,
    now: i64,
) -> Option<SkillRelationship> {
    let source = request.skill_id.as_ref()?;
    let target = request.other_skill_id.as_ref()?;
    let relationship_type = request
        .relation_type
        .unwrap_or(RelationshipType::Supersedes);
    Some(SkillRelationship {
        relation_id: format!("feedback:{feedback_id}"),
        source_skill_id: source.clone(),
        target_skill_id: target.clone(),
        relationship_type,
        vector_type: None,
        source_profile_id: request.profile_id.clone(),
        target_profile_id: request.profile_id.clone(),
        score: request.strength,
        state: if request.action.confirms() {
            RelationshipState::HumanConfirmed
        } else {
            RelationshipState::HumanRejected
        },
        evidence: serde_json::json!({
            "feedbackId": feedback_id,
            "action": request.action.as_str(),
            "sourceEvidence": request.source_evidence,
            "semantic": true,
            "pixelCoordinatesUsed": false,
        }),
        created_at: now,
        validated_at: Some(now),
    })
}

fn feedback_sample(
    feedback_id: &str,
    dataset_id: &str,
    request: &FeedbackRequest,
    now: i64,
) -> Option<ValidationSampleRecord> {
    let (task_type, label, state) = match request.action {
        FeedbackAction::ConfirmNoResult => (
            ValidationTaskType::NoResult,
            "3",
            ValidationState::HumanConfirmed,
        ),
        FeedbackAction::ForbidTrigger => (
            ValidationTaskType::Forbidden,
            "X",
            ValidationState::HumanConfirmed,
        ),
        action
            if request.skill_id.is_some()
                && (request.query_text.is_some() || request.query_id.is_some()) =>
        {
            (
                ValidationTaskType::OverallFunction,
                if action.confirms() { "3" } else { "0" },
                if action.confirms() {
                    ValidationState::HumanConfirmed
                } else {
                    ValidationState::HumanRejected
                },
            )
        }
        _ => return None,
    };
    let query = request
        .query_text
        .clone()
        .or_else(|| request.query_id.clone())?;
    let family = request
        .skill_id
        .as_deref()
        .unwrap_or(query.as_str())
        .to_string();
    let content_version = stable_hash(format!("feedback\0{feedback_id}").as_bytes());
    Some(ValidationSampleRecord {
        sample_id: format!("feedback:{feedback_id}"),
        dataset_id: dataset_id.to_string(),
        dataset_schema_version: LOCAL_DATASET_SCHEMA_VERSION.to_string(),
        dataset_content_version: content_version,
        split: split_for_family(&family).to_string(),
        skill_id: request.skill_id.clone(),
        chunk_id: None,
        query_text: query.clone(),
        query_language: "user".to_string(),
        task_type: task_type.as_str().to_string(),
        label: label.to_string(),
        state,
        evidence: serde_json::json!({
            "feedbackId": feedback_id,
            "action": request.action.as_str(),
            "sourceEvidence": request.source_evidence,
        }),
        confidence: Some(1.0),
        profile_id: request.profile_id.clone(),
        input_hash: stable_hash(query.as_bytes()),
        created_at: now,
        reviewed_at: Some(now),
    })
}

#[tauri::command]
fn revert_feedback_event(
    database: State<'_, Database>,
    feedback_id: String,
) -> Result<FeedbackEventRecord, String> {
    let event = database
        .feedback_event(&feedback_id)
        .map_err(|error| format!("无法读取反馈事件：{error}"))?
        .ok_or_else(|| "反馈事件不存在".to_string())?;
    let now = unix_timestamp_i64()?;
    let reversal_sample = event
        .after_state
        .get("queryText")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            event
                .after_state
                .get("queryId")
                .and_then(serde_json::Value::as_str)
        })
        .map(|query| {
            let query = query.to_string();
            ValidationSampleRecord {
                sample_id: format!("feedback-revert:{feedback_id}"),
                dataset_id: event.dataset_id.clone(),
                dataset_schema_version: LOCAL_DATASET_SCHEMA_VERSION.to_string(),
                dataset_content_version: stable_hash(
                    format!("feedback-revert\0{feedback_id}\0{now}").as_bytes(),
                ),
                split: split_for_family(event.skill_id.as_deref().unwrap_or(query.as_str()))
                    .to_string(),
                skill_id: event.skill_id.clone(),
                chunk_id: None,
                query_text: query.clone(),
                query_language: "user".to_string(),
                task_type: match event.action.as_str() {
                    "confirm_no_result" => "no_result",
                    "forbid_trigger" => "forbidden",
                    _ => "overall_function",
                }
                .to_string(),
                label: "U".to_string(),
                state: ValidationState::HumanReverted,
                evidence: serde_json::json!({
                    "feedbackId": feedback_id,
                    "supersedesSampleId": format!("feedback:{feedback_id}"),
                }),
                confidence: None,
                profile_id: event.profile_id.clone(),
                input_hash: stable_hash(query.as_bytes()),
                created_at: now,
                reviewed_at: Some(now),
            }
        });
    database
        .revert_feedback_bundle(&feedback_id, now, reversal_sample.as_ref())
        .map_err(|error| format!("无法撤销反馈事件：{error}"))
}

async fn run_embedding_job(app: AppHandle, job: EmbeddingJob) {
    let database = app.state::<Database>();
    if let Err(error) = run_embedding_job_inner(&app, database.inner(), &job).await {
        let _ = database.update_job_status(
            &job.job_id,
            JobStatus::Failed,
            unix_timestamp_i64().unwrap_or_default(),
            Some(&error),
        );
        if database
            .profile(&job.profile_id)
            .ok()
            .flatten()
            .is_some_and(|profile| !profile.is_active)
        {
            let _ =
                database.set_profile_status(&job.profile_id, ProfileStatus::Failed, Some(&error));
        }
        emit_toast(&app, "error", &format!("Embedding Job 失败：{error}"));
    }
}

async fn run_embedding_job_inner(
    app: &AppHandle,
    database: &Database,
    job: &EmbeddingJob,
) -> Result<(), String> {
    let context = database
        .job_context(&job.job_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Embedding Job 上下文不存在".to_string())
        .and_then(|value| {
            serde_json::from_value::<EmbeddingJobContext>(value).map_err(|error| error.to_string())
        })?;
    let profile = database
        .profile(&job.profile_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "目标 Profile 不存在".to_string())?;
    let credential_id = profile
        .credential_id
        .as_deref()
        .ok_or_else(|| "目标 Profile 没有 Embedding 凭据".to_string())?;
    let keys = read_key_metadata(app)?;
    validate_embedding_credential_by_id(
        &profile.provider,
        credential_id,
        &keys,
        profile.is_active,
    )?;
    let api_key = read_credential_secret(credential_id)?;
    if !profile.is_active {
        database
            .set_profile_status(&profile.profile_id, ProfileStatus::Building, None)
            .map_err(|error| error.to_string())?;
    }
    database
        .update_job(
            &job.job_id,
            JobStatus::Running,
            0,
            0,
            unix_timestamp_i64()?,
            None,
        )
        .map_err(|error| error.to_string())?;
    let snapshot = database
        .current_canonical_snapshot()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "规范快照不存在".to_string())?;
    let snapshot = snapshot_for_embedding(database, &snapshot)?;
    let work_snapshot = if job.kind == vectorization::JobKind::Incremental {
        incremental_work_snapshot(database, &profile.profile_id, &snapshot)?
    } else {
        snapshot.clone()
    };
    let provider = RemoteEmbeddingProvider::new(&profile, api_key, ReqwestJsonTransport);
    let analysis_provider = active_analysis_provider(app)?;
    let mut completed = 0u64;
    let mut actual_tokens = 0u64;
    let current_skill_ids = snapshot
        .skills
        .iter()
        .filter(|skill| !skill.path.starts_with("bundled://"))
        .map(|skill| skill.skill_id.clone())
        .collect::<Vec<_>>();
    let input_config = InputGenerationConfig {
        max_parent_tokens: usize::try_from(context.adaptive_policy.u)
            .unwrap_or(usize::MAX)
            .max(1),
        max_chunk_tokens: usize::try_from(context.adaptive_policy.l)
            .unwrap_or(usize::MAX)
            .max(1),
        ..Default::default()
    };

    for skill in work_snapshot
        .skills
        .iter()
        .filter(|skill| !skill.path.starts_with("bundled://"))
    {
        if database
            .is_job_cancel_requested(&job.job_id)
            .map_err(|error| error.to_string())?
        {
            database
                .update_job(
                    &job.job_id,
                    JobStatus::Cancelled,
                    completed,
                    actual_tokens,
                    unix_timestamp_i64()?,
                    None,
                )
                .map_err(|error| error.to_string())?;
            if !profile.is_active {
                database
                    .set_profile_status(&profile.profile_id, ProfileStatus::Draft, None)
                    .map_err(|error| error.to_string())?;
            }
            emit_toast(app, "info", "Embedding Job 已取消");
            return Ok(());
        }
        let prepared = prepare_skill_inputs(skill, &input_config)?;
        database
            .save_generated_inputs(
                &snapshot.snapshot_id,
                &skill.skill_id,
                &prepared.inputs,
                unix_timestamp_i64()?,
            )
            .map_err(|error| error.to_string())?;

        let rule = rule_result(&skill.skill_id, &prepared.inputs);
        let analysis_cached = database
            .has_analysis_for_input(
                &skill.skill_id,
                &rule.input_hash,
                analysis_provider.is_some(),
            )
            .map_err(|error| error.to_string())?;
        if analysis_cached {
            // Deterministic inputs and the requested analysis tier are already
            // available. Reuse them without another remote LLM call.
        } else if let Some(analysis_provider) = &analysis_provider {
            let markdown = prepared
                .inputs
                .iter()
                .map(|input| input.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let request = AnalysisRequest {
                skill_id: skill.skill_id.clone(),
                input_hash: rule.input_hash.clone(),
                markdown,
                schema_version: INPUT_SCHEMA_VERSION.to_string(),
                phase: AnalysisPhase::StructureExtraction,
            };
            match analysis_provider.analyze(request.clone()).await {
                Ok(llm) => {
                    let comparison = compare_analysis(&rule, &llm);
                    let conflict = if comparison.status == application::ComparisonStatus::Conflict {
                        match analysis_provider
                            .resolve_conflict(request, &rule, &llm)
                            .await
                        {
                            Ok(mut result) => {
                                result.comparison_id = comparison.comparison_id.clone();
                                Some(result)
                            }
                            Err(_) => None,
                        }
                    } else {
                        None
                    };
                    database
                        .save_analysis_results(
                            &rule,
                            Some(&llm),
                            Some(&comparison),
                            conflict.as_ref(),
                        )
                        .map_err(|error| error.to_string())?;
                }
                Err(_) => database
                    .save_analysis_results(&rule, None, None, None)
                    .map_err(|error| error.to_string())?,
            }
        } else {
            database
                .save_analysis_results(&rule, None, None, None)
                .map_err(|error| error.to_string())?;
        }

        let mut pending = Vec::new();
        for input in &prepared.inputs {
            if !database
                .has_ready_embedding(&profile.profile_id, &input.input_hash)
                .map_err(|error| error.to_string())?
            {
                let embedding_id = embedding_id(&profile.profile_id, &input.input_hash);
                pending.push((
                    input.text.clone(),
                    StoredVector {
                        embedding_id: embedding_id.clone(),
                        profile_id: profile.profile_id.clone(),
                        skill_id: skill.skill_id.clone(),
                        vector_type: input.parent.vector_type,
                        level: VectorLevel::Parent,
                        parent_embedding_id: None,
                        resource_category: input.parent.resource_category.clone(),
                        chunk_id: None,
                        input_hash: input.input_hash.clone(),
                        vector: Vec::new(),
                        status: VectorStatus::Ready,
                        heading_path: None,
                        source_file: Some(format!("{}/SKILL.md", skill.skill_id)),
                    },
                ));
            }
            let parent_id = embedding_id(&profile.profile_id, &input.input_hash);
            for chunk in &input.chunks {
                if !database
                    .has_ready_embedding(&profile.profile_id, &chunk.input_hash)
                    .map_err(|error| error.to_string())?
                {
                    pending.push((
                        chunk.text.clone(),
                        StoredVector {
                            embedding_id: embedding_id(&profile.profile_id, &chunk.input_hash),
                            profile_id: profile.profile_id.clone(),
                            skill_id: skill.skill_id.clone(),
                            vector_type: chunk.vector_type,
                            level: VectorLevel::Chunk,
                            parent_embedding_id: Some(parent_id.clone()),
                            resource_category: chunk.resource_category.clone(),
                            chunk_id: Some(chunk.chunk_id.clone()),
                            input_hash: chunk.input_hash.clone(),
                            vector: Vec::new(),
                            status: VectorStatus::Ready,
                            heading_path: Some(chunk.heading_path.clone()),
                            source_file: Some(chunk.relative_file_path.clone()),
                        },
                    ));
                }
            }
        }
        if !pending.is_empty() {
            let texts = pending
                .iter()
                .map(|(text, _)| text.clone())
                .collect::<Vec<_>>();
            match provider.embed(&texts).await {
                Ok(batch) => {
                    actual_tokens =
                        actual_tokens.saturating_add(batch.token_count.unwrap_or_default());
                    for ((_, mut vector), values) in pending.into_iter().zip(batch.vectors) {
                        vector.vector = values;
                        database
                            .save_embedding(&vector)
                            .map_err(|error| error.to_string())?;
                        completed += 1;
                    }
                }
                Err(error) => {
                    database
                        .expire_skill_vectors(
                            &profile.profile_id,
                            &skill.skill_id,
                            &error.to_string(),
                        )
                        .map_err(|db_error| db_error.to_string())?;
                    return Err(error.to_string());
                }
            }
        }
        database
            .update_job(
                &job.job_id,
                JobStatus::Running,
                completed,
                actual_tokens,
                unix_timestamp_i64()?,
                None,
            )
            .map_err(|error| error.to_string())?;
        emit_progress(app, job, completed);
    }
    database
        .remove_deleted_profile_skills(&profile.profile_id, &current_skill_ids)
        .map_err(|error| error.to_string())?;
    database
        .replace_indexed_skills(&profile.profile_id, &snapshot, unix_timestamp_i64()?)
        .map_err(|error| error.to_string())?;
    if job.kind == vectorization::JobKind::ProfileMigration {
        if let Some(source_profile_id) = context
            .source_profile_id
            .as_deref()
            .filter(|source| *source != profile.profile_id)
        {
            let source_relations = database
                .relationships_for_profile(source_profile_id)
                .map_err(|error| error.to_string())?;
            let target_vectors = database
                .load_vectors(&profile.profile_id)
                .map_err(|error| error.to_string())?;
            for relationship in migrate_profile_relations(
                &source_relations,
                &target_vectors,
                source_profile_id,
                &profile.profile_id,
                5,
                unix_timestamp_i64()?,
            ) {
                database
                    .save_relationship(&relationship)
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    if !profile.is_active {
        database
            .set_profile_status(&profile.profile_id, ProfileStatus::Ready, None)
            .map_err(|error| error.to_string())?;
    }
    database
        .update_job(
            &job.job_id,
            JobStatus::Completed,
            job.total_items,
            actual_tokens,
            unix_timestamp_i64()?,
            None,
        )
        .map_err(|error| error.to_string())?;
    if context.activate_on_success {
        update_embedding_role_metadata(
            app,
            || {
                database
                    .activate_profile_and_embedding_credential_atomic(
                        &profile.profile_id,
                        &context.target_credential_id,
                        &profile.provider,
                        unix_timestamp_i64()?,
                    )
                    .map_err(|error| format!("无法激活构建后的 Profile：{error}"))
            },
            &context.target_credential_id,
        )?;
    }
    let should_propose = matches!(
        job.kind,
        vectorization::JobKind::FullRebuild | vectorization::JobKind::ProfileMigration
    ) || (job.kind == vectorization::JobKind::Incremental
        && ensure_adaptive_policy(database, &profile, unix_timestamp_i64()?)?.local_update_enabled);
    if should_propose {
        let _ = create_adaptive_proposal(
            database,
            &profile,
            if job.kind == vectorization::JobKind::Incremental {
                "successful_incremental_build"
            } else {
                "successful_full_or_global_build"
            },
            unix_timestamp_i64()?,
        );
    }
    emit_progress(app, job, job.total_items);
    emit_toast(app, "success", "Embedding Job 已完成");
    Ok(())
}

fn active_analysis_provider(
    app: &AppHandle,
) -> Result<Option<RemoteAnalysisProvider<ReqwestJsonTransport>>, String> {
    let keys = read_key_metadata(app)?;
    let Some(key) = keys.iter().find(|key| key.is_agent_active) else {
        return Ok(None);
    };
    let Some(model) = key.agent_model.clone() else {
        return Ok(None);
    };
    let api_key = read_credential_secret(&key.id)?;
    Ok(Some(RemoteAnalysisProvider::new(
        key.provider.clone(),
        model,
        api_key,
        ReqwestJsonTransport,
    )))
}

fn embedding_id(profile_id: &str, input_hash: &str) -> String {
    format!(
        "embedding_{}",
        &stable_hash(format!("{profile_id}\0{input_hash}").as_bytes())[..32]
    )
}

fn emit_progress(app: &AppHandle, job: &EmbeddingJob, completed: u64) {
    let _ = app.emit(
        "embedding-job-progress",
        ProgressEvent {
            job_id: Some(job.job_id.clone()),
            label: format!("{:?}", job.kind).to_ascii_lowercase(),
            completed,
            total: job.total_items,
        },
    );
}

fn emit_toast(app: &AppHandle, kind: &str, message: &str) {
    let _ = app.emit(
        "embedding-toast",
        ToastMessage {
            id: Uuid::new_v4().to_string(),
            kind: kind.to_string(),
            message: message.to_string(),
        },
    );
}

fn user_home() -> Result<PathBuf, String> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| "无法确定当前用户主目录".to_string())
}

fn agent_configuration(agent: &str, home: &Path) -> Result<(Vec<PathBuf>, &'static str), String> {
    match agent {
        "claude-code" => Ok((
            vec![home.join(".claude").join("skills")],
            "https://code.claude.com/docs/en/skills",
        )),
        "cursor" => Ok((
            vec![
                home.join(".cursor").join("skills"),
                home.join(".agents").join("skills"),
                home.join(".claude").join("skills"),
                home.join(".codex").join("skills"),
            ],
            "https://cursor.com/docs/skills",
        )),
        "codex" => Ok((
            vec![
                home.join(".agents").join("skills"),
                // The bundled skill-installer still installs personal Skills
                // into $CODEX_HOME/skills (normally ~/.codex/skills). Keep this
                // compatibility root alongside the current ~/.agents/skills
                // convention so existing installations remain discoverable.
                home.join(".codex").join("skills"),
            ],
            "https://learn.chatgpt.com/zh-Hans/docs/build-skills",
        )),
        _ => Err("未知 Agent".to_string()),
    }
}

fn repository_ancestor_directories_from(start: &Path) -> Vec<PathBuf> {
    let mut current = start.to_path_buf();
    let mut directories = Vec::new();
    loop {
        directories.push(current.clone());
        if current.join(".git").exists() {
            return directories;
        }
        if !current.pop() {
            // Outside a repository, only the actual working directory is a
            // meaningful project scope. Do not probe arbitrary ancestors.
            return directories.into_iter().take(1).collect();
        }
    }
}

fn project_skill_roots_from(agent: &str, start: &Path) -> Vec<PathBuf> {
    let relative_roots: &[&[&str]] = match agent {
        "claude-code" => &[&[".claude", "skills"]],
        "cursor" => &[
            &[".agents", "skills"],
            &[".cursor", "skills"],
            // Cursor officially supports Claude and Codex project locations
            // for cross-agent compatibility.
            &[".claude", "skills"],
            &[".codex", "skills"],
        ],
        "codex" => &[&[".agents", "skills"]],
        _ => &[],
    };
    repository_ancestor_directories_from(start)
        .into_iter()
        .flat_map(|directory| {
            relative_roots.iter().map(move |segments| {
                segments
                    .iter()
                    .fold(directory.clone(), |path, segment| path.join(segment))
            })
        })
        .collect()
}

fn project_skill_roots(agent: &str) -> Vec<PathBuf> {
    std::env::current_dir()
        .map(|current| project_skill_roots_from(agent, &current))
        .unwrap_or_default()
}

fn normalized_path_is_within(path: &Path, root: &Path) -> bool {
    let path = normalized_identity_path(path);
    let root = normalized_identity_path(root);
    path == root
        || path
            .strip_prefix(&root)
            .is_some_and(|relative| relative.starts_with('/'))
}

fn managed_skill_roots(agent: &str, home: &Path) -> Vec<PathBuf> {
    match agent {
        "cursor" => vec![home.join(".cursor").join("skills-cursor")],
        "codex" => vec![home.join(".codex").join("skills").join(".system")],
        _ => Vec::new(),
    }
}

fn parse_skill_frontmatter(path: &Path) -> (Option<String>, Option<String>) {
    let Ok(content) = fs::read_to_string(path) else {
        return (None, None);
    };
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }

    let yaml = lines
        .take_while(|line| line.trim() != "---")
        .collect::<Vec<_>>()
        .join("\n");
    serde_yaml::from_str::<SkillFrontmatter>(&yaml)
        .map(|frontmatter| (frontmatter.name, frontmatter.description))
        .unwrap_or((None, None))
}

fn normalized_identity_path(path: &Path) -> String {
    let normalized = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let value = normalized.to_string_lossy();
    let value = value.strip_prefix(r"\\?\").unwrap_or(&value);
    if cfg!(windows) {
        value.replace('\\', "/").to_ascii_lowercase()
    } else {
        value.replace('\\', "/")
    }
}

fn read_managed_origin(directory: &Path) -> Option<ManagedSkillOrigin> {
    let content = fs::read_to_string(directory.join(MANAGED_SKILL_ORIGIN_FILE)).ok()?;
    serde_json::from_str(&content).ok()
}

fn write_managed_origin(directory: &Path, source: &Path) -> Result<(), String> {
    let source_path = normalized_identity_path(source);
    if read_managed_origin(directory).is_some_and(|origin| origin.source_path == source_path) {
        return Ok(());
    }
    let origin = ManagedSkillOrigin {
        source_path,
        managed_at: unix_timestamp()?,
    };
    let content = serde_json::to_string_pretty(&origin)
        .map_err(|error| format!("无法序列化托管 Skill 来源：{error}"))?;
    fs::write(directory.join(MANAGED_SKILL_ORIGIN_FILE), content)
        .map_err(|error| format!("无法保存托管 Skill 来源：{error}"))
}

fn collect_skill_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some(MANAGED_SKILL_ORIGIN_FILE) {
            continue;
        }
        if metadata.is_dir() {
            collect_skill_files(&path, files);
        } else if metadata.is_file() {
            files.push(path);
        }
    }
}

fn hash_skill_directory(directory: &Path) -> String {
    let mut files = Vec::new();
    collect_skill_files(directory, &mut files);
    files.sort();
    let mut hasher = Sha256::new();
    for path in files {
        if let Ok(relative) = path.strip_prefix(directory) {
            hasher.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
        }
        if let Ok(content) = fs::read(&path) {
            hasher.update(content);
        }
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn scan_skill_directory(
    directory: &Path,
    root: &Path,
    skills: &mut Vec<InstalledSkill>,
    warnings: &mut Vec<String>,
    seen_paths: &mut HashSet<PathBuf>,
    visited_directories: &mut HashSet<PathBuf>,
    scope: &str,
    is_built_in: bool,
    agent: Option<&str>,
    in_library: bool,
) {
    let normalized_directory =
        fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf());
    if !visited_directories.insert(normalized_directory.clone()) {
        return;
    }

    let skill_file = directory.join("SKILL.md");
    if directory != root && skill_file.is_file() {
        if !seen_paths.insert(normalized_directory.clone()) {
            return;
        }
        let (frontmatter_name, description) = parse_skill_frontmatter(&skill_file);
        let fallback_name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unnamed-skill")
            .to_string();
        let managed_origin = if in_library {
            read_managed_origin(directory).map(|origin| origin.source_path)
        } else {
            None
        };
        let logical_identity = managed_origin
            .clone()
            .unwrap_or_else(|| normalized_identity_path(&normalized_directory));
        let content_hash = hash_skill_directory(directory);
        skills.push(InstalledSkill {
            skill_id: stable_skill_id(&logical_identity),
            name: frontmatter_name.unwrap_or(fallback_name),
            description,
            path: normalized_directory.to_string_lossy().into_owned(),
            source_path: root.to_string_lossy().into_owned(),
            scope: scope.to_string(),
            is_built_in,
            enabled_agents: agent.into_iter().map(str::to_string).collect(),
            disabled_agents: Vec::new(),
            in_library,
            library_path: in_library.then(|| normalized_directory.to_string_lossy().into_owned()),
            backup_suppressed: false,
            managed_origin,
            content_hash,
        });
        return;
    }

    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            warnings.push(format!("无法读取 {}：{error}", directory.display()));
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if directory == root
            && path.file_name().and_then(|name| name.to_str()) == Some(MANAGED_SKILL_HISTORY_DIR)
        {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            scan_skill_directory(
                &path,
                root,
                skills,
                warnings,
                seen_paths,
                visited_directories,
                scope,
                is_built_in,
                agent,
                in_library,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_skill_root(
    root: &Path,
    skills: &mut Vec<InstalledSkill>,
    warnings: &mut Vec<String>,
    seen_paths: &mut HashSet<PathBuf>,
    scope: &str,
    is_built_in: bool,
    agent: Option<&str>,
    in_library: bool,
) {
    if !root.exists() {
        return;
    }
    let mut visited_directories = HashSet::new();
    scan_skill_directory(
        root,
        root,
        skills,
        warnings,
        seen_paths,
        &mut visited_directories,
        scope,
        is_built_in,
        agent,
        in_library,
    );
}

fn add_bundled_skill(skills: &mut Vec<InstalledSkill>, agent: &str, name: &str, description: &str) {
    let path = format!("bundled://{agent}/{name}");
    if skills
        .iter()
        .any(|skill| skill.is_built_in && skill.name.eq_ignore_ascii_case(name))
    {
        return;
    }
    let content_hash = format!("bundled:{agent}:{name}");
    skills.push(InstalledSkill {
        skill_id: stable_skill_id(&path),
        name: name.to_string(),
        description: Some(description.to_string()),
        content_hash,
        path,
        source_path: format!("{agent} official bundle"),
        scope: "system".to_string(),
        is_built_in: true,
        enabled_agents: vec![agent.to_string()],
        disabled_agents: Vec::new(),
        in_library: false,
        library_path: None,
        backup_suppressed: false,
        managed_origin: None,
    });
}

fn add_official_bundled_skills(agent: &str, skills: &mut Vec<InstalledSkill>) {
    match agent {
        "claude-code" => {
            for (name, description) in [
                ("doctor", "检查 Claude Code 安装和设置状态"),
                ("code-review", "审查代码变更并提出改进建议"),
                ("batch", "协调并行处理批量任务"),
                ("debug", "通过调试流程调查并解决问题"),
                ("loop", "按指定节奏重复执行任务"),
                ("claude-api", "协助构建 Claude API 集成"),
                ("run", "启动并操作应用以检查实际运行结果"),
                ("verify", "构建并运行应用以验证代码改动"),
                ("run-skill-generator", "生成可复用的应用运行与验证流程"),
            ] {
                add_bundled_skill(skills, agent, name, description);
            }
        }
        // Codex exposes its system Skills as real packages under
        // ~/.codex/skills/.system. Do not add a second hard-coded virtual
        // catalog here: stale virtual entries have no path that
        // [[skills.config]] can target and therefore cannot be disabled.
        "codex" => {}
        "cursor" => {
            for (name, description) in [
                ("automate", "创建由计划或外部事件触发的自动化"),
                ("babysit", "持续监控并维护 Pull Request"),
                ("canvas", "创建交互式 React 画布"),
                ("create-hook", "创建 Cursor 生命周期 Hook"),
                ("create-rule", "创建具有适当范围的 Cursor Rule"),
                ("create-skill", "创建 Agent Skill"),
                ("create-subagent", "创建专用 Subagent"),
                ("cursor-blame", "调查 AI 生成改动及来源提示"),
                ("loop", "按指定间隔重复执行 Prompt 或 Skill"),
                ("migrate-to-skills", "将规则和命令迁移为 Skills"),
                ("review", "选择并运行代码审查 Agent"),
                ("review-bugbot", "使用 Bugbot 检查缺陷与回归"),
                ("review-security", "执行安全漏洞审查"),
                ("sdk", "协助构建 Cursor SDK 集成"),
                ("shell", "将文本作为 Shell 命令执行"),
                ("split-to-prs", "将大型变更拆分为较小 PR"),
                ("statusline", "配置 Cursor CLI 状态栏"),
                ("update-cli-config", "更新 Cursor CLI 设置"),
                ("update-cursor-settings", "更新 Cursor 或 VS Code 设置"),
            ] {
                add_bundled_skill(skills, agent, name, description);
            }
        }
        _ => {}
    }
}

fn cursor_managed_counterpart(skill_path: &Path, home: &Path) -> bool {
    let portable_root = normalized_identity_path(&home.join(".agents").join("skills"));
    let normalized_skill = normalized_identity_path(skill_path);
    let Some(relative) = normalized_skill
        .strip_prefix(&portable_root)
        .and_then(|value| value.strip_prefix('/'))
    else {
        return false;
    };
    home.join(".cursor")
        .join("skills-cursor")
        .join(relative.replace('/', std::path::MAIN_SEPARATOR_STR))
        .join("SKILL.md")
        .is_file()
}

fn scan_agent_skills(agent: String) -> Result<AgentSkillsResponse, String> {
    let agent = agent.trim().to_ascii_lowercase();
    let home = user_home()?;
    let (roots, official_documentation) = agent_configuration(&agent, &home)?;
    let project_roots = project_skill_roots(&agent);
    let mut searched_paths: Vec<String> = roots
        .iter()
        .chain(project_roots.iter())
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    let mut skills = Vec::new();
    let mut warnings = Vec::new();
    let mut seen_paths = HashSet::new();

    for root in &roots {
        scan_skill_root(
            root,
            &mut skills,
            &mut warnings,
            &mut seen_paths,
            "user",
            false,
            Some(&agent),
            false,
        );
    }
    for root in &project_roots {
        scan_skill_root(
            root,
            &mut skills,
            &mut warnings,
            &mut seen_paths,
            "project",
            false,
            Some(&agent),
            false,
        );
    }

    // ~/.codex/skills is both the legacy/present installer destination and
    // the parent of the real bundled .system packages. Classify descendants
    // of .system as built-in even when they were first found through the
    // compatibility parent root.
    if agent == "codex" {
        let system_root = home.join(".codex").join("skills").join(".system");
        for skill in &mut skills {
            if normalized_path_is_within(Path::new(&skill.path), &system_root) {
                skill.is_built_in = true;
                skill.scope = "system".to_string();
                skill.source_path = system_root.to_string_lossy().into_owned();
            }
        }
    }

    // Cursor provisions some of its bundled Skills into the portable
    // ~/.agents/skills directory. They remain built-in even though the portable
    // directory is also a valid user authoring location. A matching package in
    // Cursor's managed directory is the source evidence; unrelated user Skills
    // in ~/.agents/skills remain unmarked.
    if agent == "cursor" {
        for skill in &mut skills {
            if cursor_managed_counterpart(Path::new(&skill.path), &home) {
                skill.is_built_in = true;
                skill.scope = "system".to_string();
            }
        }
    }

    let managed_roots = managed_skill_roots(&agent, &home);
    searched_paths.extend(
        managed_roots
            .iter()
            .map(|path| path.to_string_lossy().into_owned()),
    );
    for root in &managed_roots {
        let mut managed_skills = Vec::new();
        scan_skill_root(
            root,
            &mut managed_skills,
            &mut warnings,
            &mut seen_paths,
            "system",
            true,
            Some(&agent),
            false,
        );
        if agent == "cursor" {
            let portable_root = home.join(".agents").join("skills");
            let managed_root = normalized_identity_path(root);
            managed_skills.retain(|skill| {
                let path = normalized_identity_path(Path::new(&skill.path));
                let Some(relative) = path
                    .strip_prefix(&managed_root)
                    .and_then(|value| value.strip_prefix('/'))
                else {
                    return true;
                };
                !portable_root
                    .join(relative.replace('/', std::path::MAIN_SEPARATOR_STR))
                    .join("SKILL.md")
                    .is_file()
            });
        }
        skills.extend(managed_skills);
    }
    add_official_bundled_skills(&agent, &mut skills);

    searched_paths.sort();
    searched_paths.dedup();
    skills.sort_by_key(|skill| skill.name.to_lowercase());

    Ok(AgentSkillsResponse {
        agent,
        official_documentation: official_documentation.to_string(),
        searched_paths,
        skills,
        warnings,
    })
}

fn user_library_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("all_skills"))
        .map_err(|error| format!("无法确定用户 Skill 库目录：{error}"))
}

fn ensure_user_library(app: &AppHandle) -> Result<PathBuf, String> {
    let path = user_library_path(app)?;
    fs::create_dir_all(&path).map_err(|error| format!("无法创建用户 Skill 库：{error}"))?;
    Ok(path)
}

fn shadowed_cursor_portable_skill(origin: &str, home: &Path) -> bool {
    let legacy_root = normalized_identity_path(&home.join(".cursor").join("skills-cursor"));
    let portable_root = home.join(".agents").join("skills");
    let Some(relative) = origin
        .strip_prefix(&legacy_root)
        .and_then(|value| value.strip_prefix('/'))
    else {
        return false;
    };
    portable_root
        .join(relative.replace('/', std::path::MAIN_SEPARATOR_STR))
        .join("SKILL.md")
        .is_file()
}

fn archive_shadowed_cursor_library_skills(
    library: &Path,
    home: &Path,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let entries = fs::read_dir(library)
        .map_err(|error| format!("无法读取 all_skills 以迁移 Cursor 镜像：{error}"))?;
    for entry in entries.flatten() {
        let target = entry.path();
        if !target.is_dir()
            || target.file_name().and_then(|name| name.to_str()) == Some(MANAGED_SKILL_HISTORY_DIR)
        {
            continue;
        }
        let Some(origin) = read_managed_origin(&target).map(|origin| origin.source_path) else {
            continue;
        };
        if shadowed_cursor_portable_skill(&origin, home) {
            if let Err(error) = archive_managed_library_target(library, &target, &origin) {
                warnings.push(format!(
                    "无法归档被 .agents/skills 取代的 Cursor 镜像 {}：{error}",
                    target.display()
                ));
            }
        }
    }
    Ok(())
}

fn merge_skill(skills: &mut HashMap<String, InstalledSkill>, incoming: InstalledSkill) {
    let key = incoming.content_hash.clone();
    if let Some(existing) = skills.get_mut(&key) {
        if incoming
            .managed_origin
            .as_ref()
            .is_some_and(|incoming_origin| {
                existing
                    .managed_origin
                    .as_ref()
                    .is_none_or(|existing_origin| incoming_origin < existing_origin)
            })
        {
            existing.skill_id = incoming.skill_id.clone();
            existing.managed_origin = incoming.managed_origin.clone();
        }
        for agent in incoming.enabled_agents {
            if !existing.enabled_agents.contains(&agent) {
                existing.enabled_agents.push(agent);
            }
        }
        existing.enabled_agents.sort();
        existing.in_library |= incoming.in_library;
        if incoming.library_path.is_some() {
            existing.library_path = incoming.library_path;
        }
        if existing.path.starts_with("bundled://") && !incoming.path.starts_with("bundled://") {
            existing.path = incoming.path;
            existing.source_path = incoming.source_path;
        }
        existing.is_built_in |= incoming.is_built_in;
    } else {
        skills.insert(key, incoming);
    }
}

fn available_library_target(
    library: &Path,
    source: &Path,
    content_hash: &str,
) -> Result<PathBuf, String> {
    let folder_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "无法确定 Skill 目录名".to_string())?;
    let direct = library.join(folder_name);
    if !direct.exists() {
        return Ok(direct);
    }

    let short_hash = content_hash.get(..8).unwrap_or(content_hash);
    let hashed_name = format!("{folder_name}--{short_hash}");
    let hashed = library.join(&hashed_name);
    if !hashed.exists() {
        return Ok(hashed);
    }
    for index in 2..=999 {
        let candidate = library.join(format!("{hashed_name}-{index}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!("无法为 {folder_name} 分配不冲突的入库目录"))
}

fn archive_managed_library_target(
    library: &Path,
    target: &Path,
    origin: &str,
) -> Result<PathBuf, String> {
    if !target.exists() || !is_within(target, library) {
        return Err(format!(
            "托管 Skill 不在 all_skills 内：{}",
            target.display()
        ));
    }
    let history = library.join(MANAGED_SKILL_HISTORY_DIR);
    fs::create_dir_all(&history).map_err(|error| format!("无法创建 Skill 历史目录：{error}"))?;
    let origin_key = &Sha256::digest(origin.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()[..12];
    let origin_history = history.join(origin_key);
    fs::create_dir_all(&origin_history)
        .map_err(|error| format!("无法创建来源历史目录：{error}"))?;
    let folder_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "无法确定托管 Skill 目录名".to_string())?;
    let mut destination = origin_history.join(folder_name);
    for index in 1..=999 {
        if !destination.exists() {
            fs::rename(target, &destination)
                .map_err(|error| format!("无法归档旧 Skill 版本：{error}"))?;
            return Ok(destination);
        }
        destination = origin_history.join(format!("{folder_name}-{index}"));
    }
    Err(format!("无法为 {folder_name} 分配历史归档目录"))
}

fn migrate_legacy_managed_library(
    app: &AppHandle,
    library: &Path,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let audit = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定审计目录：{error}"))?
        .join("audit.jsonl");
    let Ok(content) = fs::read_to_string(audit) else {
        return Ok(());
    };
    migrate_legacy_managed_library_from_audit(library, &content, warnings)
}

fn migrate_legacy_managed_library_from_audit(
    library: &Path,
    content: &str,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let mut by_origin: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for line in content.lines() {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if record.get("action").and_then(serde_json::Value::as_str) != Some("auto_sync_to_library")
        {
            continue;
        }
        let (Some(source), Some(target)) = (
            record.get("source").and_then(serde_json::Value::as_str),
            record.get("target").and_then(serde_json::Value::as_str),
        ) else {
            continue;
        };
        let target = PathBuf::from(target);
        if target.exists() && is_within(&target, library) {
            by_origin
                .entry(normalized_identity_path(Path::new(source)))
                .or_default()
                .push(target);
        }
    }
    let active_targets = by_origin
        .values()
        .filter_map(|targets| targets.last())
        .map(|target| normalized_identity_path(target))
        .collect::<HashSet<_>>();
    for (origin, targets) in by_origin {
        let Some(active) = targets.last() else {
            continue;
        };
        if let Err(error) = write_managed_origin(active, Path::new(&origin)) {
            warnings.push(format!("无法迁移 {} 的来源记录：{error}", active.display()));
            continue;
        }
        for legacy in targets.iter().take(targets.len().saturating_sub(1)) {
            if active_targets.contains(&normalized_identity_path(legacy)) {
                continue;
            }
            if let Err(error) = archive_managed_library_target(library, legacy, &origin) {
                warnings.push(format!("无法归档旧托管版本 {}：{error}", legacy.display()));
            }
        }
    }
    Ok(())
}

fn sync_skill_to_library(
    library: &Path,
    skill: &InstalledSkill,
    existing_hashes: &mut HashSet<String>,
) -> Result<Option<PathBuf>, String> {
    if skill.path.starts_with("bundled://") || existing_hashes.contains(&skill.content_hash) {
        return Ok(None);
    }
    let source = PathBuf::from(&skill.path);
    if !source.join("SKILL.md").is_file() {
        return Ok(None);
    }
    let target = available_library_target(library, &source, &skill.content_hash)?;
    if let Err(error) = copy_directory(&source, &target) {
        let _ = fs::remove_dir_all(&target);
        return Err(error);
    }
    if let Err(error) = write_managed_origin(&target, &source) {
        let _ = fs::remove_dir_all(&target);
        return Err(error);
    }
    existing_hashes.insert(skill.content_hash.clone());
    Ok(Some(target))
}

fn auto_sync_to_library(
    app: &AppHandle,
    library: &Path,
    candidates: &[InstalledSkill],
    existing_hashes: &mut HashSet<String>,
    managed_by_origin: &mut HashMap<String, InstalledSkill>,
    excluded_skill_ids: &HashSet<String>,
    warnings: &mut Vec<String>,
) {
    for skill in candidates {
        if excluded_skill_ids.contains(&skill.skill_id) {
            continue;
        }
        let source = PathBuf::from(&skill.path);
        let origin = normalized_identity_path(&source);
        if let Some(existing) = managed_by_origin.get(&origin).cloned() {
            if existing.content_hash == skill.content_hash {
                continue;
            }
            let target = PathBuf::from(existing.library_path.unwrap_or(existing.path));
            let archived = match archive_managed_library_target(library, &target, &origin) {
                Ok(archived) => archived,
                Err(error) => {
                    warnings.push(format!("无法更新托管 Skill {}：{error}", skill.name));
                    continue;
                }
            };
            if let Err(error) = copy_directory(&source, &target)
                .and_then(|_| write_managed_origin(&target, &source))
            {
                if target.exists() {
                    let _ = fs::remove_dir_all(&target);
                }
                let restore_error = fs::rename(&archived, &target).err();
                warnings.push(format!("无法更新托管 Skill {}：{error}", skill.name));
                if let Some(restore_error) = restore_error {
                    warnings.push(format!(
                        "恢复 {} 的上一版本失败：{restore_error}",
                        skill.name
                    ));
                }
                continue;
            }
            existing_hashes.insert(skill.content_hash.clone());
            let mut updated = skill.clone();
            updated.path = target.to_string_lossy().into_owned();
            updated.library_path = Some(updated.path.clone());
            updated.in_library = true;
            updated.managed_origin = Some(origin.clone());
            managed_by_origin.insert(origin, updated);
            if let Err(error) = append_audit(app, "auto_sync_to_library", &source, Some(&target)) {
                warnings.push(format!(
                    "{} 已更新入库，但审计记录失败：{error}",
                    skill.name
                ));
            }
            continue;
        }
        let target = match sync_skill_to_library(library, skill, existing_hashes) {
            Ok(Some(target)) => target,
            Ok(None) => continue,
            Err(error) => {
                warnings.push(format!("自动入库 {} 失败：{error}", skill.name));
                continue;
            }
        };
        if let Err(error) = append_audit(app, "auto_sync_to_library", &source, Some(&target)) {
            warnings.push(format!(
                "{} 已自动入库，但审计记录失败：{error}",
                skill.name
            ));
        }
        let mut managed = skill.clone();
        managed.path = target.to_string_lossy().into_owned();
        managed.library_path = Some(managed.path.clone());
        managed.in_library = true;
        managed.managed_origin = Some(origin.clone());
        managed_by_origin.insert(origin, managed);
    }
}

fn build_canonical_snapshot(
    app: &AppHandle,
    database: &Database,
) -> Result<CanonicalSnapshot, String> {
    let scan_started_at =
        i64::try_from(unix_timestamp()?).map_err(|_| "扫描时间超出支持范围".to_string())?;
    let mut merged = HashMap::new();
    let mut agent_skills = Vec::new();
    let mut searched_paths = Vec::new();
    let mut warnings = Vec::new();

    for agent in ["claude-code", "cursor", "codex"] {
        match scan_agent_skills(agent.to_string()) {
            Ok(result) => {
                searched_paths.extend(result.searched_paths);
                warnings.extend(result.warnings);
                for skill in result.skills {
                    agent_skills.push(skill.clone());
                    merge_skill(&mut merged, skill);
                }
            }
            Err(error) => warnings.push(format!("{agent}：{error}")),
        }
    }

    let current_agent_skill_ids = agent_skills
        .iter()
        .map(|skill| skill.skill_id.clone())
        .collect::<HashSet<_>>();
    let exclusions = database
        .backup_exclusions()
        .map_err(|error| format!("无法读取停止备份记录：{error}"))?;
    let mut excluded_skill_ids = HashSet::new();
    for exclusion in exclusions {
        if current_agent_skill_ids.contains(&exclusion.skill_id) {
            if exclusion.missing_seen {
                database
                    .clear_backup_exclusion(&exclusion.skill_id)
                    .map_err(|error| format!("无法清除重新安装 Skill 的停止备份记录：{error}"))?;
            } else {
                excluded_skill_ids.insert(exclusion.skill_id);
            }
        } else {
            database
                .mark_backup_exclusion_missing(&exclusion.skill_id)
                .map_err(|error| format!("无法更新停止备份记录：{error}"))?;
            excluded_skill_ids.insert(exclusion.skill_id);
        }
    }

    let library = ensure_user_library(app)?;
    searched_paths.push(library.to_string_lossy().into_owned());
    migrate_legacy_managed_library(app, &library, &mut warnings)?;
    archive_shadowed_cursor_library_skills(&library, &user_home()?, &mut warnings)?;
    let mut library_skills = Vec::new();
    let mut library_seen = HashSet::new();
    scan_skill_root(
        &library,
        &mut library_skills,
        &mut warnings,
        &mut library_seen,
        "library",
        false,
        None,
        true,
    );
    let mut existing_hashes = library_skills
        .iter()
        .map(|skill| skill.content_hash.clone())
        .collect();
    let mut managed_by_origin = library_skills
        .iter()
        .filter_map(|skill| {
            skill
                .managed_origin
                .clone()
                .map(|origin| (origin, skill.clone()))
        })
        .collect::<HashMap<_, _>>();
    auto_sync_to_library(
        app,
        &library,
        &agent_skills,
        &mut existing_hashes,
        &mut managed_by_origin,
        &excluded_skill_ids,
        &mut warnings,
    );

    library_skills.clear();
    library_seen.clear();
    scan_skill_root(
        &library,
        &mut library_skills,
        &mut warnings,
        &mut library_seen,
        "library",
        false,
        None,
        true,
    );
    for skill in library_skills {
        merge_skill(&mut merged, skill);
    }

    searched_paths.sort();
    searched_paths.dedup();
    let mut skills: Vec<_> = merged.into_values().collect();
    skills.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.content_hash.cmp(&right.content_hash))
    });

    let mut canonical_skills = skills
        .into_iter()
        .map(canonical_skill_from_installed)
        .collect::<Result<Vec<_>, _>>()?;
    for state in database
        .agent_skill_states()
        .map_err(|error| format!("无法读取 Agent Skill 禁用状态：{error}"))?
    {
        let target_index = canonical_skills
            .iter()
            .position(|skill| skill.skill_id == state.skill_id)
            .or_else(|| {
                canonical_skills
                    .iter()
                    .position(|skill| skill.content_hash == state.skill.content_hash)
            });
        if let Some(target_index) = target_index {
            let skill = &mut canonical_skills[target_index];
            if !skill.enabled_agents.contains(&state.agent_id) {
                skill.enabled_agents.push(state.agent_id.clone());
                skill.enabled_agents.sort();
            }
            if !skill.disabled_agents.contains(&state.agent_id) {
                skill.disabled_agents.push(state.agent_id.clone());
                skill.disabled_agents.sort();
            }
        } else {
            let mut skill = state.skill;
            if !skill.enabled_agents.contains(&state.agent_id) {
                skill.enabled_agents.push(state.agent_id.clone());
            }
            skill.disabled_agents = vec![state.agent_id];
            canonical_skills.push(skill);
        }
    }
    for skill in &mut canonical_skills {
        if excluded_skill_ids.contains(&skill.skill_id) {
            skill.backup_suppressed = true;
        }
    }
    canonical_skills.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.content_hash.cmp(&right.content_hash))
    });
    let content_hash = serde_json::to_vec(&canonical_skills)
        .map(|value| {
            let digest = Sha256::digest(value);
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        })
        .map_err(|error| format!("无法计算规范快照哈希：{error}"))?;
    let scan_completed_at =
        i64::try_from(unix_timestamp()?).map_err(|_| "扫描时间超出支持范围".to_string())?;
    Ok(CanonicalSnapshot {
        snapshot_id: Uuid::new_v4().to_string(),
        content_hash,
        scan_started_at,
        scan_completed_at,
        searched_paths,
        warnings,
        skills: canonical_skills,
    })
}

fn migrate_skill_identity_v3(app: &AppHandle, database: &Database) -> Result<(), String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定 Skill 身份迁移目录：{error}"))?;
    let marker = app_data.join(SKILL_IDENTITY_V3_MARKER);
    if marker.is_file() {
        return Ok(());
    }
    let previous = database
        .current_canonical_snapshot()
        .map_err(|error| format!("无法读取身份迁移前快照：{error}"))?;
    let migrated = build_canonical_snapshot(app, database)?;
    if let Some(previous) = previous {
        let migrated_by_hash = migrated
            .skills
            .iter()
            .map(|skill| (skill.content_hash.as_str(), skill.skill_id.as_str()))
            .collect::<HashMap<_, _>>();
        for skill in &previous.skills {
            if let Some(new_id) = migrated_by_hash.get(skill.content_hash.as_str()) {
                database
                    .rebind_skill_identity(&skill.skill_id, new_id)
                    .map_err(|error| format!("无法迁移 {} 的派生数据身份：{error}", skill.name))?;
            }
        }
    }
    database
        .replace_canonical_snapshot(&migrated)
        .map_err(|error| format!("无法保存稳定身份快照：{error}"))?;
    let current_ids = migrated
        .skills
        .iter()
        .filter(|skill| !skill.path.starts_with("bundled://"))
        .map(|skill| skill.skill_id.clone())
        .collect::<Vec<_>>();
    for profile in database
        .list_profiles()
        .map_err(|error| format!("无法读取身份迁移 Profiles：{error}"))?
    {
        database
            .remove_deleted_profile_skills(&profile.profile_id, &current_ids)
            .map_err(|error| format!("无法清理 {} 的旧身份索引：{error}", profile.profile_id))?;
    }
    fs::write(&marker, b"2\n").map_err(|error| format!("无法完成 Skill 身份迁移：{error}"))
}

fn migrate_built_in_classification_v3(app: &AppHandle, database: &Database) -> Result<(), String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定内置分类迁移目录：{error}"))?;
    let marker = app_data.join(BUILT_IN_CLASSIFICATION_V3_MARKER);
    if marker.is_file() {
        return Ok(());
    }
    let snapshot = build_canonical_snapshot(app, database)?;
    database
        .replace_canonical_snapshot(&snapshot)
        .map_err(|error| format!("无法保存内置分类快照：{error}"))?;
    fs::write(&marker, b"3\n").map_err(|error| format!("无法完成内置分类迁移：{error}"))
}

fn canonical_skill_from_installed(skill: InstalledSkill) -> Result<CanonicalSkill, String> {
    let files = collect_canonical_files(&skill)?;
    Ok(CanonicalSkill {
        skill_id: skill.skill_id,
        name: skill.name,
        description: skill.description,
        path: skill.path,
        source_path: skill.source_path,
        scope: skill.scope,
        is_built_in: skill.is_built_in,
        enabled_agents: skill.enabled_agents,
        disabled_agents: skill.disabled_agents,
        in_library: skill.in_library,
        library_path: skill.library_path,
        backup_suppressed: skill.backup_suppressed,
        content_hash: skill.content_hash,
        files,
    })
}

fn collect_canonical_files(skill: &InstalledSkill) -> Result<Vec<CanonicalFile>, String> {
    if skill.path.starts_with("bundled://") {
        return Ok(Vec::new());
    }
    let root = PathBuf::from(&skill.path);
    let mut paths = Vec::new();
    collect_skill_files(&root, &mut paths);
    paths.sort();
    let mut files = Vec::new();
    for path in paths {
        let relative_path = path
            .strip_prefix(&root)
            .map_err(|error| format!("无法计算 {} 的相对路径：{error}", path.display()))?
            .to_string_lossy()
            .replace('\\', "/");
        let metadata = fs::metadata(&path)
            .map_err(|error| format!("无法读取 {} 的元数据：{error}", path.display()))?;
        let content = fs::read(&path)
            .map_err(|error| format!("无法读取 {} 以计算哈希：{error}", path.display()))?;
        let content_hash = Sha256::digest(&content)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());
        let is_embeddable = extension.as_deref().is_some_and(is_embeddable_extension);
        let media_type = extension.as_deref().map(media_type_for_extension);
        let semantic_role = semantic_role_for_path(&relative_path);
        let modified_at = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .and_then(|value| i64::try_from(value.as_secs()).ok());
        let file_identity = format!("{}\0{relative_path}", skill.skill_id);
        files.push(CanonicalFile {
            file_id: format!(
                "file_{}",
                &Sha256::digest(file_identity.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()[..32]
            ),
            relative_path,
            media_type: media_type.map(str::to_string),
            extension,
            content_hash,
            size_bytes: i64::try_from(metadata.len()).unwrap_or(i64::MAX),
            is_embeddable,
            modified_at,
            semantic_role,
        });
    }
    Ok(files)
}

fn is_embeddable_extension(extension: &str) -> bool {
    matches!(
        extension,
        "md" | "txt"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
            | "rs"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "py"
            | "sh"
            | "ps1"
            | "html"
            | "css"
            | "sql"
            | "xml"
            | "csv"
    )
}

fn media_type_for_extension(extension: &str) -> &'static str {
    match extension {
        "md" => "text/markdown",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "html" => "text/html",
        "css" => "text/css",
        "csv" => "text/csv",
        "xml" => "application/xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        extension if is_embeddable_extension(extension) => "text/plain",
        _ => "application/octet-stream",
    }
}

fn semantic_role_for_path(relative_path: &str) -> Option<String> {
    let lower = relative_path.to_ascii_lowercase();
    if lower == "skill.md" {
        Some("skill_definition".to_string())
    } else if lower.starts_with("scripts/") {
        Some("scripts".to_string())
    } else if lower.starts_with("references/") {
        Some("references".to_string())
    } else if lower.starts_with("examples/") {
        Some("examples".to_string())
    } else if lower.starts_with("assets/") {
        Some("assets".to_string())
    } else {
        None
    }
}

fn installed_skill_from_canonical(skill: &CanonicalSkill) -> InstalledSkill {
    InstalledSkill {
        skill_id: skill.skill_id.clone(),
        name: skill.name.clone(),
        description: skill.description.clone(),
        path: skill.path.clone(),
        source_path: skill.source_path.clone(),
        scope: skill.scope.clone(),
        is_built_in: skill.is_built_in,
        enabled_agents: skill.enabled_agents.clone(),
        disabled_agents: skill.disabled_agents.clone(),
        in_library: skill.in_library,
        library_path: skill.library_path.clone(),
        backup_suppressed: skill.backup_suppressed,
        managed_origin: None,
        content_hash: skill.content_hash.clone(),
    }
}

fn response_from_snapshot(
    snapshot: &CanonicalSnapshot,
    agent: &str,
    official_documentation: String,
) -> AgentSkillsResponse {
    let skills = snapshot
        .skills
        .iter()
        .filter(|skill| agent == "all" || skill.enabled_agents.iter().any(|item| item == agent))
        .map(installed_skill_from_canonical)
        .collect();
    AgentSkillsResponse {
        agent: agent.to_string(),
        official_documentation,
        searched_paths: snapshot.searched_paths.clone(),
        skills,
        warnings: snapshot.warnings.clone(),
    }
}

fn get_or_build_canonical_snapshot<F>(
    database: &Database,
    builder: F,
) -> Result<CanonicalSnapshot, String>
where
    F: FnOnce() -> Result<CanonicalSnapshot, String>,
{
    if let Some(snapshot) = database
        .current_canonical_snapshot()
        .map_err(|error| format!("无法读取规范快照：{error}"))?
    {
        return Ok(snapshot);
    }
    let snapshot = builder()?;
    database
        .replace_canonical_snapshot(&snapshot)
        .map_err(|error| format!("无法保存规范快照：{error}"))?;
    Ok(snapshot)
}

fn load_or_refresh_canonical_snapshot(
    app: &AppHandle,
    database: &Database,
) -> Result<CanonicalSnapshot, String> {
    get_or_build_canonical_snapshot(database, || build_canonical_snapshot(app, database))
}

#[tauri::command]
fn refresh_canonical_skills_snapshot(
    app: AppHandle,
    database: State<'_, Database>,
) -> Result<CanonicalSnapshot, String> {
    let snapshot = build_canonical_snapshot(&app, database.inner())?;
    database
        .replace_canonical_snapshot(&snapshot)
        .map_err(|error| format!("无法保存规范快照：{error}"))?;
    Ok(snapshot)
}

#[tauri::command]
fn get_canonical_skills_snapshot(
    app: AppHandle,
    database: State<'_, Database>,
) -> Result<CanonicalSnapshot, String> {
    load_or_refresh_canonical_snapshot(&app, database.inner())
}

#[tauri::command]
fn list_all_skills(
    app: AppHandle,
    database: State<'_, Database>,
) -> Result<AgentSkillsResponse, String> {
    let snapshot = load_or_refresh_canonical_snapshot(&app, database.inner())?;
    Ok(response_from_snapshot(&snapshot, "all", String::new()))
}

#[tauri::command]
fn list_agent_skills(
    app: AppHandle,
    database: State<'_, Database>,
    agent: String,
) -> Result<AgentSkillsResponse, String> {
    let agent = agent.trim().to_ascii_lowercase();
    let home = user_home()?;
    let (_, official_documentation) = agent_configuration(&agent, &home)?;
    let snapshot = load_or_refresh_canonical_snapshot(&app, database.inner())?;
    Ok(response_from_snapshot(
        &snapshot,
        &agent,
        official_documentation.to_string(),
    ))
}

fn canonical_skill<'a>(
    snapshot: &'a CanonicalSnapshot,
    skill_id: &str,
) -> Result<&'a CanonicalSkill, String> {
    snapshot
        .skills
        .iter()
        .find(|skill| skill.skill_id == skill_id)
        .ok_or_else(|| "未找到目标 Skill，请刷新列表后重试".to_string())
}

fn resolve_agent_matches(
    skill: &CanonicalSkill,
    candidates: Vec<InstalledSkill>,
    action: &str,
) -> Vec<InstalledSkill> {
    let physical = candidates
        .into_iter()
        .filter(|candidate| !candidate.path.starts_with("bundled://"))
        .collect::<Vec<_>>();
    let mut exact = physical
        .iter()
        .filter(|candidate| {
            candidate.content_hash == skill.content_hash
                || normalized_identity_path(Path::new(&candidate.path))
                    == normalized_identity_path(Path::new(&skill.path))
        })
        .cloned()
        .collect::<Vec<_>>();
    exact.sort_by(|left, right| left.path.cmp(&right.path));
    exact.dedup_by(|left, right| {
        normalized_identity_path(Path::new(&left.path))
            == normalized_identity_path(Path::new(&right.path))
    });
    if !exact.is_empty() || action != "disable" {
        return exact;
    }

    // Disabling is reversible and only writes a path-scoped config entry. If
    // an older canonical snapshot has a stale hash, a unique name match lets
    // Codex still target the real current package. Never use this fallback for
    // uninstall, where deleting the wrong same-named directory is unacceptable.
    let named = physical
        .into_iter()
        .filter(|candidate| candidate.name.eq_ignore_ascii_case(&skill.name))
        .collect::<Vec<_>>();
    (named.len() == 1).then_some(named).unwrap_or_default()
}

fn physical_agent_matches(
    skill: &CanonicalSkill,
    agent: &str,
    action: &str,
) -> Vec<InstalledSkill> {
    scan_agent_skills(agent.to_string())
        .map(|response| resolve_agent_matches(skill, response.skills, action))
        .unwrap_or_default()
}

fn agents_for_physical_path(path: &Path) -> Vec<String> {
    let target = normalized_identity_path(path);
    let mut agents = ["claude-code", "cursor", "codex"]
        .into_iter()
        .filter(|agent| {
            scan_agent_skills((*agent).to_string())
                .map(|response| response.skills)
                .unwrap_or_default()
                .iter()
                .filter(|candidate| !candidate.path.starts_with("bundled://"))
                .any(|candidate| normalized_identity_path(Path::new(&candidate.path)) == target)
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    agents.sort();
    agents.dedup();
    agents
}

fn path_looks_like_plugin(path: &Path) -> bool {
    normalized_identity_path(path).contains("/plugins/")
}

fn action_plan_for(
    database: &Database,
    skill_id: &str,
    view_id: &str,
    action: &str,
) -> Result<SkillActionPlan, String> {
    let snapshot = database
        .current_canonical_snapshot()
        .map_err(|error| format!("无法读取规范快照：{error}"))?
        .ok_or_else(|| "当前没有 Skills 快照".to_string())?;
    let skill = canonical_skill(&snapshot, skill_id)?;
    if view_id == "all" {
        let target_paths = skill.library_path.clone().into_iter().collect::<Vec<_>>();
        return Ok(SkillActionPlan {
            skill_id: skill.skill_id.clone(),
            skill_name: skill.name.clone(),
            view_id: view_id.to_string(),
            action: action.to_string(),
            allowed: action == "uninstall" && !target_paths.is_empty(),
            reason: (target_paths.is_empty()).then(|| {
                if skill.backup_suppressed {
                    "该 Skill 的后备副本已删除，并已停止自动备份".to_string()
                } else {
                    "该 Skill 当前没有 all_skills 后备副本".to_string()
                }
            }),
            target_paths,
            affected_agents: Vec::new(),
            shared_installation: false,
            plugin_operation: false,
            symbolic_link_only: false,
            keeps_library_copy: false,
            keeps_other_agents: !skill.enabled_agents.is_empty(),
        });
    }
    if !matches!(view_id, "claude-code" | "cursor" | "codex") {
        return Err("未知 Skill 分类".to_string());
    }
    let matches = physical_agent_matches(skill, view_id, action);
    let target_paths = matches
        .iter()
        .map(|item| item.path.clone())
        .collect::<Vec<_>>();
    let mut affected_agents = target_paths
        .iter()
        .flat_map(|path| agents_for_physical_path(Path::new(path)))
        .collect::<Vec<_>>();
    affected_agents.sort();
    affected_agents.dedup();
    let plugin_operation = target_paths
        .iter()
        .any(|path| path_looks_like_plugin(Path::new(path)));
    let symbolic_link_only = !target_paths.is_empty()
        && target_paths.iter().all(|path| {
            fs::symlink_metadata(path)
                .map(|metadata| metadata.file_type().is_symlink())
                .unwrap_or(false)
        });
    let unsupported_builtin = skill.is_built_in
        && action == "disable"
        && target_paths.is_empty()
        && view_id != "claude-code";
    let cursor_shared_disable = action == "disable"
        && view_id == "cursor"
        && affected_agents.iter().any(|agent| agent != "cursor");
    let reason = if skill.is_built_in && action == "uninstall" {
        Some("该内置 Skill 由 Agent 管理，不能作为普通目录单独卸载".to_string())
    } else if unsupported_builtin {
        Some("该内置或虚拟 Skill 没有可独立管理的安装目录".to_string())
    } else if plugin_operation && action == "uninstall" {
        Some("该 Skill 由插件提供；请通过 Agent 的插件管理器卸载整个插件".to_string())
    } else if cursor_shared_disable {
        Some("Cursor 与其他 Agent 共用此安装目录，移动目录无法只禁用 Cursor".to_string())
    } else if target_paths.is_empty() && action == "disable" && view_id != "claude-code" {
        Some("没有解析到该 Skill 的实体目录，无法写入 Agent 的路径级禁用配置".to_string())
    } else if target_paths.is_empty() && action == "uninstall" {
        Some("没有找到属于当前 Agent 的可卸载实体目录".to_string())
    } else {
        None
    };
    Ok(SkillActionPlan {
        skill_id: skill.skill_id.clone(),
        skill_name: skill.name.clone(),
        view_id: view_id.to_string(),
        action: action.to_string(),
        allowed: reason.is_none(),
        reason,
        target_paths,
        affected_agents: affected_agents.clone(),
        shared_installation: affected_agents.len() > 1,
        plugin_operation,
        symbolic_link_only,
        keeps_library_copy: skill.in_library,
        keeps_other_agents: skill.enabled_agents.iter().any(|agent| agent != view_id),
    })
}

#[tauri::command]
fn prepare_skill_action(
    database: State<'_, Database>,
    skill_id: String,
    view_id: String,
    action: String,
) -> Result<SkillActionPlan, String> {
    action_plan_for(database.inner(), &skill_id, &view_id, &action)
}

fn remove_installation(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("无法读取卸载目标 {}：{error}", path.display()))?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path).map_err(|error| format!("移除链接 {} 失败：{error}", path.display()))
    } else if metadata.is_dir() {
        fs::remove_dir_all(path).map_err(|error| format!("卸载 {} 失败：{error}", path.display()))
    } else {
        Err(format!("不支持的卸载目标：{}", path.display()))
    }
}

#[tauri::command]
fn uninstall_skill(
    app: AppHandle,
    database: State<'_, Database>,
    skill_id: String,
    view_id: String,
) -> Result<CanonicalSnapshot, String> {
    let plan = action_plan_for(database.inner(), &skill_id, &view_id, "uninstall")?;
    if !plan.allowed {
        return Err(plan
            .reason
            .unwrap_or_else(|| "该 Skill 不能卸载".to_string()));
    }
    let snapshot = database
        .current_canonical_snapshot()
        .map_err(|error| format!("无法读取规范快照：{error}"))?
        .ok_or_else(|| "当前没有 Skills 快照".to_string())?;
    let skill = canonical_skill(&snapshot, &skill_id)?.clone();
    if view_id == "all" {
        let library = ensure_user_library(&app)?;
        if plan
            .target_paths
            .iter()
            .any(|target| !is_within(Path::new(target), &library))
        {
            return Err("后备副本路径已变化或不在 all_skills 中，操作已取消".to_string());
        }
        database
            .suppress_skill_backup(
                &skill.skill_id,
                &skill.content_hash,
                &skill.source_path,
                unix_timestamp_i64()?,
            )
            .map_err(|error| format!("无法保存停止备份记录：{error}"))?;
    }
    for target in &plan.target_paths {
        let path = PathBuf::from(target);
        remove_installation(&path)?;
        append_audit(&app, &format!("uninstall_from_{view_id}"), &path, None)?;
    }
    let refreshed = build_canonical_snapshot(&app, database.inner())?;
    database
        .replace_canonical_snapshot(&refreshed)
        .map_err(|error| format!("无法保存卸载后的快照：{error}"))?;
    Ok(refreshed)
}

#[tauri::command]
fn restore_skill_backup(
    app: AppHandle,
    database: State<'_, Database>,
    skill_id: String,
) -> Result<CanonicalSnapshot, String> {
    database
        .clear_backup_exclusion(&skill_id)
        .map_err(|error| format!("无法恢复自动备份：{error}"))?;
    let refreshed = build_canonical_snapshot(&app, database.inner())?;
    database
        .replace_canonical_snapshot(&refreshed)
        .map_err(|error| format!("无法保存重新备份后的快照：{error}"))?;
    Ok(refreshed)
}

fn write_atomic(path: &Path, content: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法创建配置目录：{error}"))?;
    }
    let temporary = path.with_extension(format!("deadalus-{}.tmp", Uuid::new_v4()));
    fs::write(&temporary, content).map_err(|error| format!("无法写入临时配置：{error}"))?;
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("无法原子替换配置：{error}")
    })
}

fn set_claude_skill_override(
    home: &Path,
    name: &str,
    disabled: bool,
) -> Result<Option<serde_json::Value>, String> {
    let path = home.join(".claude").join("settings.json");
    let mut root = if path.is_file() {
        serde_json::from_slice::<serde_json::Value>(
            &fs::read(&path).map_err(|error| format!("无法读取 Claude Code 设置：{error}"))?,
        )
        .map_err(|error| format!("Claude Code 设置不是有效 JSON：{error}"))?
    } else {
        serde_json::json!({})
    };
    let object = root
        .as_object_mut()
        .ok_or_else(|| "Claude Code 设置根节点必须是对象".to_string())?;
    let overrides = object
        .entry("skillOverrides")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| "Claude Code skillOverrides 必须是对象".to_string())?;
    let previous = overrides.get(name).cloned();
    if disabled {
        overrides.insert(
            name.to_string(),
            serde_json::Value::String("off".to_string()),
        );
    } else {
        overrides.remove(name);
        if overrides.is_empty() {
            object.remove("skillOverrides");
        }
    }
    let serialized = serde_json::to_vec_pretty(&root).map_err(|error| error.to_string())?;
    write_atomic(&path, &serialized)?;
    Ok(previous)
}

fn codex_config_path(home: &Path) -> PathBuf {
    home.join(".codex").join("config.toml")
}

fn set_codex_skill_enabled(home: &Path, skill_path: &str, enabled: bool) -> Result<(), String> {
    let path = codex_config_path(home);
    let content = fs::read_to_string(&path).unwrap_or_default();
    let mut document = content
        .parse::<DocumentMut>()
        .map_err(|error| format!("Codex config.toml 格式无效：{error}"))?;
    if !document.contains_key("skills") {
        document["skills"] = Item::Table(Table::new());
    }
    let skills = document["skills"]
        .as_table_mut()
        .ok_or_else(|| "Codex config.toml 中 skills 必须是表".to_string())?;
    if !skills.contains_key("config") {
        skills.insert("config", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let entries = skills
        .get_mut("config")
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| "Codex config.toml 中 skills.config 必须是表数组".to_string())?;
    let existing_index = entries
        .iter()
        .position(|entry| entry.get("path").and_then(Item::as_str) == Some(skill_path));
    if let Some(existing_index) = existing_index {
        let entry = entries
            .get_mut(existing_index)
            .expect("existing Codex skill config");
        entry.insert("enabled", Item::Value(Value::from(enabled)));
    } else {
        let mut entry = Table::new();
        entry.insert("path", Item::Value(Value::from(skill_path)));
        entry.insert("enabled", Item::Value(Value::from(enabled)));
        entries.push(entry);
    }
    write_atomic(&path, document.to_string().as_bytes())
}

fn cursor_disabled_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("disabled_skills").join("cursor"))
        .map_err(|error| format!("无法确定 Cursor 停用目录：{error}"))
}

#[tauri::command]
fn disable_skill_for_agent(
    app: AppHandle,
    database: State<'_, Database>,
    skill_id: String,
    agent: String,
) -> Result<CanonicalSnapshot, String> {
    let plan = action_plan_for(database.inner(), &skill_id, &agent, "disable")?;
    if !plan.allowed {
        return Err(plan
            .reason
            .unwrap_or_else(|| "该 Skill 不能按当前方式禁用".to_string()));
    }
    let snapshot = database
        .current_canonical_snapshot()
        .map_err(|error| format!("无法读取规范快照：{error}"))?
        .ok_or_else(|| "当前没有 Skills 快照".to_string())?;
    let skill = canonical_skill(&snapshot, &skill_id)?.clone();
    let home = user_home()?;
    let (method, installation_path, parked_path, previous_value) = match agent.as_str() {
        "claude-code" => {
            let previous = set_claude_skill_override(&home, &skill.name, true)?;
            (
                "claude_skill_override".to_string(),
                skill.path.clone(),
                None,
                previous,
            )
        }
        "codex" => {
            let target = plan
                .target_paths
                .first()
                .cloned()
                .ok_or_else(|| "没有可配置的 Codex Skill 路径".to_string())?;
            let skill_file = PathBuf::from(&target).join("SKILL.md");
            set_codex_skill_enabled(&home, &skill_file.to_string_lossy(), false)?;
            ("codex_config".to_string(), target, None, None)
        }
        "cursor" => {
            let target = plan
                .target_paths
                .first()
                .cloned()
                .ok_or_else(|| "没有可移动的 Cursor Skill 路径".to_string())?;
            let source = PathBuf::from(&target);
            let root = cursor_disabled_root(&app)?;
            fs::create_dir_all(&root)
                .map_err(|error| format!("无法创建 Cursor 停用目录：{error}"))?;
            let destination = root.join(format!("{}-{}", skill.skill_id, Uuid::new_v4()));
            fs::rename(&source, &destination)
                .map_err(|error| format!("无法移动 Cursor Skill 到停用目录：{error}"))?;
            (
                "cursor_park".to_string(),
                target,
                Some(destination.to_string_lossy().into_owned()),
                None,
            )
        }
        _ => return Err("未知 Agent".to_string()),
    };
    database
        .upsert_agent_skill_state(&AgentSkillState {
            skill_id: skill.skill_id.clone(),
            agent_id: agent.clone(),
            installation_path,
            method,
            parked_path,
            skill,
            previous_value,
            disabled_at: unix_timestamp_i64()?,
        })
        .map_err(|error| format!("无法保存禁用状态：{error}"))?;
    append_audit(
        &app,
        &format!("disable_for_{agent}"),
        Path::new(&skill_id),
        None,
    )?;
    let refreshed = build_canonical_snapshot(&app, database.inner())?;
    database
        .replace_canonical_snapshot(&refreshed)
        .map_err(|error| format!("无法保存禁用后的快照：{error}"))?;
    Ok(refreshed)
}

#[tauri::command]
fn enable_skill_for_agent(
    app: AppHandle,
    database: State<'_, Database>,
    skill_id: String,
    agent: String,
) -> Result<CanonicalSnapshot, String> {
    let state = database
        .agent_skill_state(&skill_id, &agent)
        .map_err(|error| format!("无法读取禁用状态：{error}"))?
        .ok_or_else(|| "该 Skill 当前没有禁用记录".to_string())?;
    let home = user_home()?;
    match state.method.as_str() {
        "claude_skill_override" => {
            set_claude_skill_override(&home, &state.skill.name, false)?;
            if let Some(previous) = &state.previous_value {
                if let Some(value) = previous.as_str() {
                    if value != "off" {
                        let path = home.join(".claude").join("settings.json");
                        let mut root = serde_json::from_slice::<serde_json::Value>(
                            &fs::read(&path).map_err(|error| error.to_string())?,
                        )
                        .map_err(|error| error.to_string())?;
                        root.as_object_mut()
                            .unwrap()
                            .entry("skillOverrides")
                            .or_insert_with(|| serde_json::json!({}))
                            .as_object_mut()
                            .unwrap()
                            .insert(
                                state.skill.name.clone(),
                                serde_json::Value::String(value.to_string()),
                            );
                        write_atomic(
                            &path,
                            &serde_json::to_vec_pretty(&root).map_err(|error| error.to_string())?,
                        )?;
                    }
                }
            }
        }
        "codex_config" => {
            let skill_file = PathBuf::from(&state.installation_path).join("SKILL.md");
            set_codex_skill_enabled(&home, &skill_file.to_string_lossy(), true)?;
        }
        "cursor_park" => {
            let parked = state
                .parked_path
                .as_deref()
                .ok_or_else(|| "Cursor 停用记录缺少移动路径".to_string())?;
            if Path::new(&state.installation_path).exists() {
                return Err("Cursor 原安装位置已被占用，未覆盖现有内容".to_string());
            }
            fs::rename(parked, &state.installation_path)
                .map_err(|error| format!("无法恢复 Cursor Skill：{error}"))?;
        }
        _ => return Err("未知禁用方式".to_string()),
    }
    database
        .delete_agent_skill_state(&skill_id, &agent)
        .map_err(|error| format!("无法清除禁用状态：{error}"))?;
    append_audit(
        &app,
        &format!("enable_for_{agent}"),
        Path::new(&state.installation_path),
        None,
    )?;
    let refreshed = build_canonical_snapshot(&app, database.inner())?;
    database
        .replace_canonical_snapshot(&refreshed)
        .map_err(|error| format!("无法保存解禁后的快照：{error}"))?;
    Ok(refreshed)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchPreferences {
    include_disabled_skills: bool,
}

#[tauri::command]
fn get_search_preferences(database: State<'_, Database>) -> Result<SearchPreferences, String> {
    Ok(SearchPreferences {
        include_disabled_skills: database
            .include_disabled_search()
            .map_err(|error| format!("无法读取搜索设置：{error}"))?,
    })
}

#[tauri::command]
fn set_include_disabled_skills(
    database: State<'_, Database>,
    enabled: bool,
) -> Result<SearchPreferences, String> {
    database
        .set_include_disabled_search(enabled)
        .map_err(|error| format!("无法保存搜索设置：{error}"))?;
    Ok(SearchPreferences {
        include_disabled_skills: enabled,
    })
}

fn copy_directory(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        return Err(format!("目标已存在：{}", target.display()));
    }
    fs::create_dir_all(target).map_err(|error| format!("无法创建目标目录：{error}"))?;

    let entries = fs::read_dir(source).map_err(|error| format!("无法读取源目录：{error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("无法读取源条目：{error}"))?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)
            .map_err(|error| format!("无法读取文件元数据：{error}"))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("暂不复制符号链接：{}", source_path.display()));
        }
        if metadata.is_dir() {
            copy_directory(&source_path, &target_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &target_path)
                .map_err(|error| format!("复制 {} 失败：{error}", source_path.display()))?;
        }
    }
    Ok(())
}

fn is_within(path: &Path, root: &Path) -> bool {
    let Ok(path) = fs::canonicalize(path) else {
        return false;
    };
    let Ok(root) = fs::canonicalize(root) else {
        return false;
    };
    path != root && path.starts_with(root)
}

fn agent_user_root(agent: &str, home: &Path) -> Result<PathBuf, String> {
    match agent {
        "claude-code" => Ok(home.join(".claude").join("skills")),
        "cursor" => Ok(home.join(".cursor").join("skills")),
        "codex" => Ok(home.join(".agents").join("skills")),
        _ => Err("未知 Agent".to_string()),
    }
}

fn append_audit(
    app: &AppHandle,
    action: &str,
    source: &Path,
    target: Option<&Path>,
) -> Result<(), String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法确定审计目录：{error}"))?;
    fs::create_dir_all(&directory).map_err(|error| format!("无法创建审计目录：{error}"))?;
    let record = serde_json::json!({
        "timestamp": unix_timestamp()?,
        "action": action,
        "source": source.to_string_lossy(),
        "target": target.map(|path| path.to_string_lossy())
    });
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("audit.jsonl"))
        .map_err(|error| format!("无法打开审计日志：{error}"))?;
    writeln!(file, "{record}").map_err(|error| format!("无法写入审计日志：{error}"))
}

#[tauri::command]
fn copy_library_skill_to_agent(
    app: AppHandle,
    skill_path: String,
    agent: String,
) -> Result<(), String> {
    let source = PathBuf::from(skill_path);
    let library = ensure_user_library(&app)?;
    if !is_within(&source, &library) || !source.join("SKILL.md").is_file() {
        return Err("只能从用户 all_skills 库安装有效 Skill".to_string());
    }

    let home = user_home()?;
    let target_root = agent_user_root(&agent, &home)?;
    fs::create_dir_all(&target_root).map_err(|error| format!("无法创建 Agent 目录：{error}"))?;
    let folder_name = source
        .file_name()
        .ok_or_else(|| "无法确定 Skill 目录名".to_string())?;
    let target = target_root.join(folder_name);
    if let Err(error) = copy_directory(&source, &target) {
        let _ = fs::remove_dir_all(&target);
        return Err(error);
    }
    append_audit(&app, &format!("copy_to_{agent}"), &source, Some(&target))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let database_path = app
                .path()
                .app_data_dir()
                .map_err(|error| std::io::Error::other(format!("无法确定数据库目录：{error}")))?
                .join("deadalus.sqlite");
            let database = database::Database::open(database_path)
                .map_err(|error| std::io::Error::other(format!("无法初始化数据库：{error}")))?;
            let recovered_at = unix_timestamp_i64().map_err(std::io::Error::other)?;
            database
                .recover_interrupted_work(recovered_at)
                .map_err(|error| {
                    std::io::Error::other(format!("无法恢复中断的本地任务：{error}"))
                })?;
            let library = ensure_user_library(app.handle()).map_err(std::io::Error::other)?;
            migrate_skill_identity_v3(app.handle(), &database).map_err(std::io::Error::other)?;
            migrate_built_in_classification_v3(app.handle(), &database)
                .map_err(std::io::Error::other)?;
            let (event_sender, event_receiver) = mpsc::channel();
            let mut watcher = notify::recommended_watcher(move |result: notify::Result<_>| {
                if result.is_ok() {
                    let _ = event_sender.send(());
                }
            })
            .map_err(|error| {
                std::io::Error::other(format!("无法创建 all_skills 监听器：{error}"))
            })?;
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                while event_receiver.recv().is_ok() {
                    let mut disconnected = false;
                    loop {
                        match event_receiver.recv_timeout(Duration::from_secs(1)) {
                            Ok(()) => {}
                            Err(mpsc::RecvTimeoutError::Timeout) => break,
                            Err(mpsc::RecvTimeoutError::Disconnected) => {
                                disconnected = true;
                                break;
                            }
                        }
                    }
                    let _ = app_handle.emit("snapshot-invalidated", ());
                    let _ = app_handle.emit("all-skills-changed", ());
                    if disconnected {
                        break;
                    }
                }
            });
            watcher
                .watch(&library, RecursiveMode::Recursive)
                .map_err(|error| std::io::Error::other(format!("无法监听 all_skills：{error}")))?;
            // Watch every existing user/project compatibility root as well.
            // Optional roots may not exist yet and must not prevent startup.
            let home = user_home().map_err(std::io::Error::other)?;
            let mut watched_paths = HashSet::new();
            watched_paths.insert(normalized_identity_path(&library));
            for agent in ["claude-code", "cursor", "codex"] {
                let (user_roots, _) =
                    agent_configuration(agent, &home).map_err(std::io::Error::other)?;
                for root in user_roots
                    .into_iter()
                    .chain(project_skill_roots(agent))
                    .chain(managed_skill_roots(agent, &home))
                {
                    if root.exists() && watched_paths.insert(normalized_identity_path(&root)) {
                        let _ = watcher.watch(&root, RecursiveMode::Recursive);
                    }
                }
            }
            app.manage(LibraryWatcher {
                _watcher: Mutex::new(watcher),
            });
            app.manage(database);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_api_keys,
            save_api_key,
            activate_api_key,
            activate_api_key_for_purpose,
            delete_api_key,
            set_agent_model,
            list_provider_models,
            get_embedding_profile_defaults,
            list_embedding_profiles,
            delete_embedding_profile,
            create_embedding_profile,
            get_active_embedding_profile,
            activate_ready_embedding_profile,
            list_embedding_adaptive_history,
            set_local_adaptive_enabled,
            propose_adaptive_settings,
            apply_adaptive_history,
            reject_adaptive_history,
            revert_adaptive_history,
            get_embedding_profile_settings,
            prepare_profile_change,
            begin_profile_rebuild_preflight,
            begin_profile_migration_preflight,
            begin_full_rebuild_preflight,
            get_index_sync_status,
            set_index_auto_update,
            set_ignore_built_in_skills,
            scan_embedding_changes,
            get_index_diff,
            list_embedding_jobs,
            delete_embedding_job_history,
            clear_embedding_job_history,
            start_embedding_job,
            cancel_embedding_job,
            semantic_search,
            get_search_preferences,
            set_include_disabled_skills,
            list_skill_relations,
            get_skill_graph,
            generate_local_validation_samples,
            list_local_validation_samples,
            run_local_validation,
            list_local_validation_runs,
            export_local_validation_data,
            reset_local_validation_data,
            delete_local_validation_sample,
            record_feedback_event,
            revert_feedback_event,
            list_agent_skills,
            list_all_skills,
            refresh_canonical_skills_snapshot,
            get_canonical_skills_snapshot,
            prepare_skill_action,
            uninstall_skill,
            disable_skill_for_agent,
            enable_skill_for_agent,
            restore_skill_backup,
            copy_library_skill_to_agent
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("deadalus-{label}-{unique}"));
        fs::create_dir_all(&path).expect("temporary directory should be created");
        path
    }

    fn test_canonical_skill(skill_id: &str, name: &str, enabled_agents: &[&str]) -> CanonicalSkill {
        CanonicalSkill {
            skill_id: skill_id.to_string(),
            name: name.to_string(),
            description: None,
            path: format!("skills/{name}"),
            source_path: "skills".to_string(),
            scope: "user".to_string(),
            is_built_in: false,
            enabled_agents: enabled_agents
                .iter()
                .map(|agent| (*agent).to_string())
                .collect(),
            disabled_agents: Vec::new(),
            in_library: true,
            library_path: Some(format!("library/{name}")),
            backup_suppressed: false,
            content_hash: format!("{name}-hash"),
            files: Vec::new(),
        }
    }

    fn test_canonical_snapshot() -> CanonicalSnapshot {
        CanonicalSnapshot {
            snapshot_id: "snapshot-test".to_string(),
            content_hash: "snapshot-hash".to_string(),
            scan_started_at: 1,
            scan_completed_at: 2,
            searched_paths: vec!["skills".to_string()],
            warnings: Vec::new(),
            skills: vec![
                test_canonical_skill("skill-codex", "codex-only", &["codex"]),
                test_canonical_skill("skill-cursor", "cursor-only", &["cursor"]),
                test_canonical_skill("skill-shared", "shared", &["claude-code", "cursor"]),
            ],
        }
    }

    fn test_key(id: &str, provider: &str) -> ApiKeyMetadata {
        ApiKeyMetadata {
            id: id.to_string(),
            provider: provider.to_string(),
            masked_key: "••••test".to_string(),
            saved_at: 1,
            is_agent_active: false,
            is_embedding_active: false,
            agent_model: None,
            legacy_is_active: None,
        }
    }

    fn test_embedding_profile(
        id: &str,
        provider: &str,
        model: &str,
        version: &str,
        dimensions: u32,
        credential_id: &str,
    ) -> EmbeddingProfile {
        EmbeddingProfile {
            profile_id: id.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            model_version: version.to_string(),
            dimensions,
            input_schema_version: INPUT_SCHEMA_VERSION.to_string(),
            chunk_policy_version: CHUNK_POLICY_VERSION.to_string(),
            tokenizer: None,
            credential_id: Some(credential_id.to_string()),
            status: ProfileStatus::Ready,
            is_active: false,
            created_at: 1,
            activated_at: None,
            error: None,
        }
    }

    #[test]
    fn incremental_work_snapshot_keeps_only_added_or_changed_skills() {
        let database = Database::in_memory().unwrap();
        let profile = test_embedding_profile(
            "profile-incremental",
            "openai",
            "text-embedding-3-small",
            "1",
            1536,
            "credential-openai",
        );
        database.upsert_profile(&profile).unwrap();
        let snapshot = test_canonical_snapshot();
        database
            .replace_indexed_skills(&profile.profile_id, &snapshot, 1)
            .unwrap();

        let unchanged = incremental_work_snapshot(&database, &profile.profile_id, &snapshot)
            .expect("unchanged snapshot should be reusable");
        assert!(unchanged.skills.is_empty());

        let mut updated = snapshot.clone();
        updated.skills[0].content_hash = "changed-content".to_string();
        updated
            .skills
            .push(test_canonical_skill("skill-new", "new", &["codex"]));
        let work = incremental_work_snapshot(&database, &profile.profile_id, &updated)
            .expect("changed snapshot should produce incremental work");
        assert_eq!(
            work.skills
                .iter()
                .map(|skill| skill.skill_id.as_str())
                .collect::<Vec<_>>(),
            vec!["skill-codex", "skill-new"]
        );
    }

    #[test]
    fn virtual_bundled_catalogs_are_exposed_only_when_no_real_system_directory_exists() {
        let mut claude = Vec::new();
        add_official_bundled_skills("claude-code", &mut claude);
        assert!(claude.iter().all(|skill| skill.is_built_in));
        assert!(claude.iter().any(|skill| skill.name == "doctor"));

        let mut codex = Vec::new();
        add_official_bundled_skills("codex", &mut codex);
        assert!(codex.is_empty());

        let mut cursor = Vec::new();
        add_official_bundled_skills("cursor", &mut cursor);
        assert_eq!(cursor.len(), 19);
        assert!(cursor.iter().all(|skill| skill.is_built_in));
        assert!(cursor.iter().any(|skill| skill.name == "review-security"));
    }

    #[test]
    fn codex_disable_can_resolve_a_unique_current_path_from_a_stale_snapshot() {
        let mut canonical = test_canonical_skill("skill-docs", "OpenAI Docs", &["codex"]);
        canonical.path = "bundled://codex/openai-docs".to_string();
        canonical.content_hash = "stale-hash".to_string();
        canonical.is_built_in = true;
        let mut candidate = installed_skill_from_canonical(&canonical);
        candidate.path = "C:/Users/demo/.codex/skills/.system/openai-docs".to_string();
        candidate.content_hash = "current-hash".to_string();

        let resolved = resolve_agent_matches(&canonical, vec![candidate.clone()], "disable");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, candidate.path);
        assert!(resolve_agent_matches(&canonical, vec![candidate], "uninstall").is_empty());
    }

    #[test]
    fn codex_disable_rejects_ambiguous_same_named_paths() {
        let mut canonical = test_canonical_skill("skill-docs", "OpenAI Docs", &["codex"]);
        canonical.content_hash = "stale-hash".to_string();
        let mut first = installed_skill_from_canonical(&canonical);
        first.path = "C:/one/openai-docs".to_string();
        first.content_hash = "first-hash".to_string();
        let mut second = first.clone();
        second.path = "C:/two/openai-docs".to_string();
        second.content_hash = "second-hash".to_string();

        assert!(resolve_agent_matches(&canonical, vec![first, second], "disable").is_empty());
    }

    #[test]
    fn cursor_managed_skills_are_included_when_installed() {
        let Ok(home) = user_home() else {
            return;
        };
        let managed_root = home.join(".cursor").join("skills-cursor");
        if !managed_root.exists() {
            return;
        }

        let response = scan_agent_skills("cursor".to_string()).expect("cursor scan should succeed");
        let managed = response
            .skills
            .iter()
            .filter(|skill| {
                skill.is_built_in && skill.source_path == managed_root.to_string_lossy()
            })
            .collect::<Vec<_>>();
        assert!(managed.iter().all(|skill| {
            !shadowed_cursor_portable_skill(
                &normalized_identity_path(Path::new(&skill.path)),
                &home,
            )
        }));
    }

    #[test]
    fn cursor_provisioned_portable_skills_are_built_in_but_user_skills_are_not() {
        let home = temporary_directory("cursor-provisioned-classification");
        let portable = home.join(".agents").join("skills");
        let managed = home.join(".cursor").join("skills-cursor");
        let provisioned = portable.join("canvas");
        let user = portable.join("my-private-workflow");
        fs::create_dir_all(&provisioned).unwrap();
        fs::create_dir_all(&user).unwrap();
        fs::create_dir_all(managed.join("canvas")).unwrap();
        fs::write(provisioned.join("SKILL.md"), "---\nname: canvas\n---\n").unwrap();
        fs::write(
            user.join("SKILL.md"),
            "---\nname: my-private-workflow\n---\n",
        )
        .unwrap();
        fs::write(
            managed.join("canvas").join("SKILL.md"),
            "---\nname: canvas\n---\n",
        )
        .unwrap();

        assert!(cursor_managed_counterpart(&provisioned, &home));
        assert!(!cursor_managed_counterpart(&user, &home));

        let mut skills = Vec::new();
        let mut warnings = Vec::new();
        let mut seen = HashSet::new();
        scan_skill_root(
            &portable,
            &mut skills,
            &mut warnings,
            &mut seen,
            "user",
            false,
            Some("cursor"),
            false,
        );
        for skill in &mut skills {
            if cursor_managed_counterpart(Path::new(&skill.path), &home) {
                skill.is_built_in = true;
                skill.scope = "system".to_string();
            }
        }
        add_official_bundled_skills("cursor", &mut skills);
        let canvas = skills
            .iter()
            .filter(|skill| skill.name == "canvas")
            .collect::<Vec<_>>();
        assert_eq!(canvas.len(), 1);
        assert!(canvas[0].is_built_in);
        assert!(
            !skills
                .iter()
                .find(|skill| skill.name == "my-private-workflow")
                .unwrap()
                .is_built_in
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn user_skill_roots_include_legacy_and_cross_agent_compatibility_locations() {
        let home = PathBuf::from("C:/Users/test");
        let (codex, _) = agent_configuration("codex", &home).unwrap();
        assert_eq!(
            codex,
            vec![
                home.join(".agents").join("skills"),
                home.join(".codex").join("skills"),
            ]
        );

        let (cursor, _) = agent_configuration("cursor", &home).unwrap();
        assert!(cursor.contains(&home.join(".cursor").join("skills")));
        assert!(cursor.contains(&home.join(".agents").join("skills")));
        assert!(cursor.contains(&home.join(".claude").join("skills")));
        assert!(cursor.contains(&home.join(".codex").join("skills")));

        let (claude, _) = agent_configuration("claude-code", &home).unwrap();
        assert_eq!(claude, vec![home.join(".claude").join("skills")]);
    }

    #[test]
    fn project_roots_follow_agent_official_and_compatibility_conventions() {
        let root = temporary_directory("project-skill-roots");
        fs::create_dir_all(root.join(".git")).unwrap();
        let nested = root.join("apps").join("desktop");
        fs::create_dir_all(&nested).unwrap();

        let codex = project_skill_roots_from("codex", &nested);
        assert!(codex.contains(&root.join(".agents").join("skills")));
        assert!(!codex.contains(&root.join(".codex").join("skills")));

        let claude = project_skill_roots_from("claude-code", &nested);
        assert!(claude.contains(&root.join(".claude").join("skills")));

        let cursor = project_skill_roots_from("cursor", &nested);
        for agent_directory in [".agents", ".cursor", ".claude", ".codex"] {
            assert!(cursor.contains(&root.join(agent_directory).join("skills")));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn codex_system_root_is_distinct_from_similarly_prefixed_directories() {
        let root = PathBuf::from("C:/Users/test/.codex/skills/.system");
        assert!(normalized_path_is_within(&root.join("openai-docs"), &root));
        assert!(!normalized_path_is_within(
            &PathBuf::from("C:/Users/test/.codex/skills/.system-copy/example"),
            &root
        ));
    }

    #[test]
    fn legacy_key_metadata_migrates_without_losing_role_access() {
        let (metadata, migrated) = parse_key_metadata(
            r#"[{
                "id": "openai-test",
                "provider": "openai",
                "maskedKey": "••••1234",
                "savedAt": 1,
                "isActive": true
            }]"#,
        )
        .expect("legacy metadata should migrate");
        assert!(migrated);
        assert!(metadata[0].is_agent_active);
        assert!(metadata[0].is_embedding_active);
        assert!(metadata[0].legacy_is_active.is_none());
        let serialized = serde_json::to_value(&metadata[0]).unwrap();
        assert!(serialized.get("isActive").is_none());
        assert_eq!(serialized["isAgentActive"], true);
        assert_eq!(serialized["isEmbeddingActive"], true);
    }

    #[test]
    fn credential_roles_activate_independently_and_allow_dual_use() {
        let mut keys = vec![
            test_key("openai", "openai"),
            test_key("anthropic", "anthropic"),
            test_key("qwen", "qwen"),
        ];
        activate_metadata_for_purpose(&mut keys, "anthropic", CredentialPurpose::Agent).unwrap();
        activate_metadata_for_purpose(&mut keys, "openai", CredentialPurpose::Embedding).unwrap();
        assert!(keys[1].is_agent_active);
        assert!(keys[0].is_embedding_active);
        assert!(!keys[1].is_embedding_active);

        activate_metadata_for_purpose(&mut keys, "openai", CredentialPurpose::Agent).unwrap();
        assert!(keys[0].is_agent_active);
        assert!(keys[0].is_embedding_active);
        assert!(!keys[1].is_agent_active);
    }

    #[test]
    fn non_embedding_provider_is_rejected_for_embedding_role() {
        let mut keys = vec![test_key("anthropic", "anthropic")];
        assert!(activate_metadata_for_purpose(
            &mut keys,
            "anthropic",
            CredentialPurpose::Embedding
        )
        .is_err());
        assert!(!keys[0].is_embedding_active);
    }

    #[test]
    fn profile_credentials_require_active_matching_embedding_provider() {
        let mut openai = test_key("openai", "openai");
        openai.is_embedding_active = true;
        let mut qwen = test_key("qwen", "qwen");
        qwen.is_embedding_active = true;
        let anthropic = test_key("anthropic", "anthropic");
        let keys = vec![openai, qwen, anthropic];
        assert!(validate_embedding_credential("openai", "openai", &keys).is_ok());
        assert!(validate_embedding_credential("qwen", "openai", &keys).is_err());
        assert!(validate_embedding_credential("anthropic", "anthropic", &keys).is_err());
    }

    #[test]
    fn staged_credentials_preserve_active_state_and_switch_only_on_success() {
        let database = Database::in_memory().unwrap();
        let old = test_embedding_profile(
            "active",
            "openai",
            "text-embedding-3-small",
            "1",
            1536,
            "old-key",
        );
        database.upsert_profile(&old).unwrap();
        database.activate_profile_atomic("active", 2).unwrap();
        database
            .activate_credential_binding("old-key", "embedding", "openai", 2)
            .unwrap();
        let mut old_key = test_key("old-key", "openai");
        old_key.is_embedding_active = true;
        let new_key = test_key("new-key", "openai");
        let keys = vec![old_key, new_key];
        let request = ProfileChangeRequest {
            source_profile_id: Some("active".to_string()),
            target_profile_id: None,
            target_credential_id: Some("new-key".to_string()),
            provider: "openai".to_string(),
            model: None,
            reason: "credential".to_string(),
        };
        let staged = stage_embedding_job_target(
            &database,
            &keys,
            JobStrategy::ProfileMigration,
            Some(&request),
            None,
            3,
        )
        .unwrap();
        assert!(staged.key_rotation_only);
        assert_eq!(
            database
                .active_profile()
                .unwrap()
                .unwrap()
                .credential_id
                .as_deref(),
            Some("old-key")
        );
        assert!(database
            .rotate_active_profile_credential_atomic("active", "new-key", "qwen", 4)
            .is_err());
        assert_eq!(
            database
                .active_credential_binding("embedding")
                .unwrap()
                .unwrap()
                .credential_id,
            "old-key"
        );
        database
            .rotate_active_profile_credential_atomic("active", "new-key", "openai", 5)
            .unwrap();
        assert_eq!(
            database
                .active_profile()
                .unwrap()
                .unwrap()
                .credential_id
                .as_deref(),
            Some("new-key")
        );
        assert_eq!(
            database
                .active_credential_binding("embedding")
                .unwrap()
                .unwrap()
                .credential_id,
            "new-key"
        );
    }

    #[test]
    fn full_rebuild_clones_active_profile_and_migration_stages_inactive_credential() {
        let database = Database::in_memory().unwrap();
        let active = test_embedding_profile(
            "active",
            "openai",
            "text-embedding-3-small",
            "1",
            1536,
            "openai-key",
        );
        database.upsert_profile(&active).unwrap();
        database.activate_profile_atomic("active", 2).unwrap();
        let mut openai = test_key("openai-key", "openai");
        openai.is_embedding_active = true;
        let qwen = test_key("qwen-key", "qwen");
        let keys = vec![openai, qwen];

        let rebuild = stage_embedding_job_target(
            &database,
            &keys,
            JobStrategy::FullRebuild,
            None,
            Some("active"),
            3,
        )
        .unwrap();
        assert_ne!(rebuild.profile.profile_id, "active");
        assert!(!rebuild.profile.is_active);
        assert_eq!(rebuild.profile.status, ProfileStatus::Draft);
        assert_eq!(
            database.active_profile().unwrap().unwrap().profile_id,
            "active"
        );

        let request = ProfileChangeRequest {
            source_profile_id: Some("active".to_string()),
            target_profile_id: None,
            target_credential_id: Some("qwen-key".to_string()),
            provider: "qwen".to_string(),
            model: None,
            reason: "profile".to_string(),
        };
        let migration = stage_embedding_job_target(
            &database,
            &keys,
            JobStrategy::ProfileMigration,
            Some(&request),
            None,
            4,
        )
        .unwrap();
        assert_eq!(migration.profile.provider, "qwen");
        assert_eq!(migration.profile.credential_id.as_deref(), Some("qwen-key"));
        assert!(!keys[1].is_embedding_active);
        assert_eq!(
            database.active_profile().unwrap().unwrap().profile_id,
            "active"
        );
    }

    #[test]
    fn validation_run_is_advisory_pending_and_preserves_effective_version() {
        let sample = ValidationSampleRecord {
            sample_id: "sample-versioned".to_string(),
            dataset_id: "user-local:snapshot".to_string(),
            dataset_schema_version: LOCAL_DATASET_SCHEMA_VERSION.to_string(),
            dataset_content_version: "content-v1".to_string(),
            split: "validation".to_string(),
            skill_id: Some("skill-a".to_string()),
            chunk_id: None,
            query_text: "find skill a".to_string(),
            query_language: "en".to_string(),
            task_type: "overall_function".to_string(),
            label: "3".to_string(),
            state: ValidationState::MachineConsensus,
            evidence: serde_json::json!({"provisional": true, "gold": false}),
            confidence: Some(0.6),
            profile_id: Some("profile".to_string()),
            input_hash: "input".to_string(),
            created_at: 1,
            reviewed_at: None,
        };
        let (dataset_id, content_version, effective) = effective_validation_dataset(&[sample]);
        let run = pending_validation_run("profile".to_string(), dataset_id, content_version, 2);
        assert_eq!(run.status, "pending");
        assert_eq!(effective.len(), 1);
        assert_eq!(run.metrics["advisoryOnly"], true);
        assert_eq!(run.metrics["blocksActivation"], false);
        assert!(!run.dataset_content_version.is_empty());
    }

    #[test]
    fn rejected_semantic_feedback_creates_reversible_human_label() {
        let request = FeedbackRequest {
            dataset_id: Some("user-local:test".to_string()),
            skill_id: Some("skill-a".to_string()),
            query_id: None,
            query_text: Some("do not return skill a".to_string()),
            other_skill_id: Some("skill-b".to_string()),
            relation_type: Some(RelationshipType::SimilarTo),
            action: FeedbackAction::RejectRelation,
            strength: Some(1.0),
            profile_id: Some("profile".to_string()),
            source_evidence: serde_json::json!({"explicit": true}),
        };
        request.validate().unwrap();
        let sample = feedback_sample("feedback", "user-local:test", &request, 1).unwrap();
        assert_eq!(sample.state, ValidationState::HumanRejected);
        assert_eq!(sample.label, "0");
        let relation = feedback_relationship("feedback", &request, 1).unwrap();
        assert_eq!(relation.state, RelationshipState::HumanRejected);
    }

    #[test]
    fn adaptive_toggle_proposal_apply_reject_and_revert_preserve_history() {
        let database = Database::in_memory().unwrap();
        let profile = test_embedding_profile(
            "adaptive-profile",
            "openai",
            "text-embedding-3-small",
            "1",
            1536,
            "credential",
        );
        database.upsert_profile(&profile).unwrap();
        let initial = ensure_adaptive_policy(&database, &profile, 1).unwrap();
        assert!(!initial.local_update_enabled);
        let toggled =
            set_local_adaptive_enabled_impl(&database, "adaptive-profile", true, 2).unwrap();
        assert!(toggled.local_update_enabled);

        let proposal = propose_adaptive_settings_impl(&database, "adaptive-profile", 3).unwrap();
        let proposal_value = parse_adaptive_proposal(&proposal).unwrap();
        assert!(proposal_value.requires_review);
        assert_eq!(
            proposal_value.effective_for,
            "jobs_created_after_explicit_apply"
        );
        assert_eq!(
            database
                .adaptive_policy("adaptive-profile")
                .unwrap()
                .unwrap()
                .m,
            initial.m
        );

        let applied = apply_adaptive_history_impl(&database, &proposal.history_id, 4).unwrap();
        assert_eq!(
            applied.applied_history_id.as_deref(),
            Some(proposal.history_id.as_str())
        );
        assert_eq!(applied.effective_after, 4);
        assert!(applied.local_update_enabled);
        let restored = revert_adaptive_history_impl(&database, &proposal.history_id, 5).unwrap();
        assert_eq!(restored.m, toggled.m);
        assert!(restored.local_update_enabled);

        let rejected = propose_adaptive_settings_impl(&database, "adaptive-profile", 6).unwrap();
        let rejected_record =
            reject_adaptive_history_impl(&database, &rejected.history_id).unwrap();
        assert_eq!(rejected_record.decision, "rejected");
        assert!(database.adaptive_history("adaptive-profile").unwrap().len() >= 3);
    }

    #[test]
    fn local_validation_export_and_reset_are_explicit_and_in_memory() {
        let database = Database::in_memory().unwrap();
        let profile = test_embedding_profile(
            "validation-profile",
            "openai",
            "text-embedding-3-small",
            "1",
            1536,
            "credential",
        );
        database.upsert_profile(&profile).unwrap();
        database
            .save_validation_sample(&ValidationSampleRecord {
                sample_id: "local-sample".to_string(),
                dataset_id: "user-local:test".to_string(),
                dataset_schema_version: LOCAL_DATASET_SCHEMA_VERSION.to_string(),
                dataset_content_version: "v1".to_string(),
                split: "validation".to_string(),
                skill_id: Some("skill".to_string()),
                chunk_id: None,
                query_text: "local query".to_string(),
                query_language: "en".to_string(),
                task_type: "overall_function".to_string(),
                label: "3".to_string(),
                state: ValidationState::RuleDerived,
                evidence: serde_json::json!({"local": true}),
                confidence: Some(0.8),
                profile_id: Some("validation-profile".to_string()),
                input_hash: "hash".to_string(),
                created_at: 1,
                reviewed_at: None,
            })
            .unwrap();
        let export =
            build_local_validation_export(&database, Some("validation-profile"), 2).unwrap();
        assert_eq!(export["localOnly"], true);
        assert_eq!(export["externalWritePerformed"], false);
        assert_eq!(export["samples"].as_array().unwrap().len(), 1);
        assert!(database.delete_validation_sample("local-sample").unwrap());
        assert!(!database.delete_validation_sample("local-sample").unwrap());

        database
            .save_validation_sample(
                &serde_json::from_value::<ValidationSampleRecord>(export["samples"][0].clone())
                    .unwrap(),
            )
            .unwrap();
        let reset = database
            .reset_local_validation_data(Some("validation-profile"))
            .unwrap();
        assert_eq!(reset.samples, 1);
        assert!(database
            .list_validation_samples(Some("validation-profile"), None)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn provider_model_parser_accepts_common_response_shapes() {
        let openai = parse_provider_models(&serde_json::json!({
            "data": [
                {"id": "gpt-b", "owned_by": "provider"},
                {"id": "gpt-a", "owned_by": "provider"}
            ]
        }))
        .unwrap();
        assert_eq!(openai[0].id, "gpt-a");
        let anthropic = parse_provider_models(&serde_json::json!({
            "models": [{"id": "claude-test", "display_name": "Claude Test"}]
        }))
        .unwrap();
        assert_eq!(anthropic[0].display_name, "Claude Test");
    }

    #[test]
    fn stable_skill_ids_follow_logical_identity_not_content() {
        let logical_identity = "c:/users/test/.agents/skills/example";
        assert_eq!(
            stable_skill_id(logical_identity),
            stable_skill_id(logical_identity)
        );
        assert_ne!(
            stable_skill_id(logical_identity),
            stable_skill_id("c:/users/test/.agents/skills/other")
        );
        assert!(stable_skill_id(logical_identity).starts_with("skill_"));
    }

    #[test]
    fn canonical_get_reuses_persisted_snapshot_without_rescan() {
        let database = Database::in_memory().unwrap();
        let first = get_or_build_canonical_snapshot(&database, || Ok(test_canonical_snapshot()))
            .expect("initial build should persist");
        let second = get_or_build_canonical_snapshot(&database, || {
            panic!("persisted canonical snapshot must not trigger a rescan")
        })
        .expect("persisted snapshot should load");
        assert_eq!(first, second);
    }

    #[test]
    fn canonical_agent_filter_matches_membership_filtering() {
        let snapshot = test_canonical_snapshot();
        let response = response_from_snapshot(&snapshot, "cursor", "docs".to_string());
        let expected = snapshot
            .skills
            .iter()
            .filter(|skill| skill.enabled_agents.iter().any(|agent| agent == "cursor"))
            .map(|skill| skill.skill_id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            response
                .skills
                .iter()
                .map(|skill| skill.skill_id.clone())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(response
            .skills
            .iter()
            .all(|skill| skill.enabled_agents.iter().any(|agent| agent == "cursor")));
    }

    #[test]
    fn bounded_recursive_scan_stops_at_skill_boundary() {
        let root = temporary_directory("bounded-scan");
        let skill = root.join("documents").join("pdf-reader");
        fs::create_dir_all(skill.join("examples").join("nested"))
            .expect("nested directories should be created");
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: pdf-reader\ndescription: Reads PDFs\n---\n",
        )
        .expect("skill should be written");
        fs::write(
            skill.join("examples").join("nested").join("SKILL.md"),
            "---\nname: nested-example\ndescription: Example only\n---\n",
        )
        .expect("nested example should be written");

        let mut skills = Vec::new();
        let mut warnings = Vec::new();
        let mut seen = HashSet::new();
        scan_skill_root(
            &root,
            &mut skills,
            &mut warnings,
            &mut seen,
            "library",
            false,
            None,
            true,
        );

        assert!(warnings.is_empty());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "pdf-reader");
        fs::remove_dir_all(root).expect("temporary directory should be removed");
    }

    #[test]
    fn identical_copies_merge_and_keep_all_agent_memberships() {
        let root = temporary_directory("dedup");
        let claude_root = root.join("claude");
        let cursor_root = root.join("cursor");
        for skill in [claude_root.join("shared"), cursor_root.join("shared")] {
            fs::create_dir_all(&skill).expect("skill directory should be created");
            fs::write(
                skill.join("SKILL.md"),
                "---\nname: shared\ndescription: Shared workflow\n---\n",
            )
            .expect("skill should be written");
        }

        let mut discovered = Vec::new();
        let mut warnings = Vec::new();
        let mut seen = HashSet::new();
        scan_skill_root(
            &claude_root,
            &mut discovered,
            &mut warnings,
            &mut seen,
            "user",
            false,
            Some("claude-code"),
            false,
        );
        scan_skill_root(
            &cursor_root,
            &mut discovered,
            &mut warnings,
            &mut seen,
            "user",
            false,
            Some("cursor"),
            false,
        );

        assert_ne!(discovered[0].skill_id, discovered[1].skill_id);
        let first_id = discovered[0].skill_id.clone();
        let mut merged = HashMap::new();
        for skill in discovered {
            merge_skill(&mut merged, skill);
        }
        let skill = merged.into_values().next().expect("skill should exist");
        assert_eq!(skill.skill_id, first_id);
        assert_eq!(skill.enabled_agents, vec!["claude-code", "cursor"]);
        fs::remove_dir_all(root).expect("temporary directory should be removed");
    }

    #[test]
    fn directory_copy_preserves_package_and_refuses_overwrite() {
        let root = temporary_directory("copy");
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(source.join("references")).expect("source should be created");
        fs::write(source.join("SKILL.md"), "---\nname: copy-test\n---\n")
            .expect("skill should be written");
        fs::write(source.join("references").join("guide.md"), "guide")
            .expect("reference should be written");

        copy_directory(&source, &target).expect("first copy should succeed");
        assert_eq!(
            fs::read_to_string(target.join("references").join("guide.md"))
                .expect("copied reference should exist"),
            "guide"
        );
        assert!(copy_directory(&source, &target).is_err());
        fs::remove_dir_all(root).expect("temporary directory should be removed");
    }

    #[test]
    fn automatic_sync_archives_distinct_same_named_skills_without_overwrite() {
        let root = temporary_directory("automatic-sync");
        let library = root.join("library");
        let claude_root = root.join("claude");
        let cursor_root = root.join("cursor");
        fs::create_dir_all(&library).expect("library should be created");
        for (skill, description) in [
            (claude_root.join("shared"), "Claude behavior"),
            (cursor_root.join("shared"), "Cursor behavior"),
        ] {
            fs::create_dir_all(&skill).expect("skill should be created");
            fs::write(
                skill.join("SKILL.md"),
                format!("---\nname: shared\ndescription: {description}\n---\n"),
            )
            .expect("skill should be written");
        }

        let mut discovered = Vec::new();
        let mut warnings = Vec::new();
        let mut seen = HashSet::new();
        for (agent, source) in [("claude-code", &claude_root), ("cursor", &cursor_root)] {
            scan_skill_root(
                source,
                &mut discovered,
                &mut warnings,
                &mut seen,
                "user",
                false,
                Some(agent),
                false,
            );
        }

        let mut all_catalog = HashMap::new();
        for skill in discovered.iter().cloned() {
            merge_skill(&mut all_catalog, skill);
        }
        assert!(discovered
            .iter()
            .all(|skill| all_catalog.contains_key(&skill.content_hash)));

        let mut hashes = HashSet::new();
        for skill in &discovered {
            assert!(sync_skill_to_library(&library, skill, &mut hashes)
                .expect("automatic sync should succeed")
                .is_some());
        }
        assert!(sync_skill_to_library(&library, &discovered[0], &mut hashes)
            .expect("repeat sync should succeed")
            .is_none());

        let mut archived = Vec::new();
        let mut archived_seen = HashSet::new();
        scan_skill_root(
            &library,
            &mut archived,
            &mut warnings,
            &mut archived_seen,
            "library",
            false,
            None,
            true,
        );
        assert_eq!(archived.len(), 2);
        assert_eq!(hashes.len(), 2);
        fs::remove_dir_all(root).expect("temporary directory should be removed");
    }

    #[test]
    fn managed_skill_update_keeps_identity_and_history_out_of_active_scan() {
        let root = temporary_directory("managed-update");
        let source_root = root.join("agent");
        let source = source_root.join("shared");
        let library = root.join("library");
        fs::create_dir_all(&source).expect("source should be created");
        fs::create_dir_all(&library).expect("library should be created");
        fs::write(
            source.join("SKILL.md"),
            "---\nname: shared\ndescription: Version one\n---\n",
        )
        .expect("first version should be written");

        let mut source_skills = Vec::new();
        let mut warnings = Vec::new();
        let mut seen = HashSet::new();
        scan_skill_root(
            &source_root,
            &mut source_skills,
            &mut warnings,
            &mut seen,
            "user",
            false,
            Some("codex"),
            false,
        );
        let stable_id = source_skills[0].skill_id.clone();
        let mut hashes = HashSet::new();
        let target = sync_skill_to_library(&library, &source_skills[0], &mut hashes)
            .unwrap()
            .unwrap();

        let mut library_skills = Vec::new();
        seen.clear();
        scan_skill_root(
            &library,
            &mut library_skills,
            &mut warnings,
            &mut seen,
            "library",
            false,
            None,
            true,
        );
        assert_eq!(library_skills[0].skill_id, stable_id);

        fs::write(
            source.join("SKILL.md"),
            "---\nname: shared\ndescription: Version two\n---\n",
        )
        .expect("second version should be written");
        source_skills.clear();
        seen.clear();
        scan_skill_root(
            &source_root,
            &mut source_skills,
            &mut warnings,
            &mut seen,
            "user",
            false,
            Some("codex"),
            false,
        );
        assert_eq!(source_skills[0].skill_id, stable_id);
        assert_ne!(
            source_skills[0].content_hash,
            library_skills[0].content_hash
        );

        let archived =
            archive_managed_library_target(&library, &target, &normalized_identity_path(&source))
                .expect("old version should be archived");
        assert!(archived.join("SKILL.md").is_file());
        copy_directory(&source, &target).expect("new version should replace active copy");
        write_managed_origin(&target, &source).expect("origin should be preserved");

        library_skills.clear();
        seen.clear();
        scan_skill_root(
            &library,
            &mut library_skills,
            &mut warnings,
            &mut seen,
            "library",
            false,
            None,
            true,
        );
        assert_eq!(library_skills.len(), 1);
        assert_eq!(library_skills[0].skill_id, stable_id);
        assert_eq!(
            library_skills[0].content_hash,
            source_skills[0].content_hash
        );
        fs::remove_dir_all(root).expect("temporary directory should be removed");
    }

    #[test]
    fn legacy_audit_migration_archives_old_versions_and_marks_latest() {
        let root = temporary_directory("legacy-managed-migration");
        let library = root.join("library");
        let source = root.join("agent").join("shared");
        let old = library.join("shared");
        let latest = library.join("shared--newhash");
        for (directory, description) in [(&old, "Old"), (&latest, "Latest")] {
            fs::create_dir_all(directory).expect("legacy target should be created");
            fs::write(
                directory.join("SKILL.md"),
                format!("---\nname: shared\ndescription: {description}\n---\n"),
            )
            .expect("legacy target should contain a Skill");
        }
        let records = [
            serde_json::json!({
                "action": "auto_sync_to_library",
                "source": source,
                "target": old,
                "timestamp": 1
            }),
            serde_json::json!({
                "action": "auto_sync_to_library",
                "source": source,
                "target": latest,
                "timestamp": 2
            }),
        ]
        .into_iter()
        .map(|record| record.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        let mut warnings = Vec::new();
        migrate_legacy_managed_library_from_audit(&library, &records, &mut warnings)
            .expect("legacy migration should succeed");
        assert!(warnings.is_empty());
        assert!(!old.exists());
        assert!(latest.join(MANAGED_SKILL_ORIGIN_FILE).is_file());

        let mut skills = Vec::new();
        let mut seen = HashSet::new();
        scan_skill_root(
            &library,
            &mut skills,
            &mut warnings,
            &mut seen,
            "library",
            false,
            None,
            true,
        );
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].description.as_deref(), Some("Latest"));
        assert!(library.join(MANAGED_SKILL_HISTORY_DIR).is_dir());
        fs::remove_dir_all(root).expect("temporary directory should be removed");
    }

    #[test]
    fn portable_agents_skill_shadows_legacy_cursor_mirror_and_backup() {
        let root = temporary_directory("cursor-shadow");
        let home = root.join("home");
        let legacy = home.join(".cursor").join("skills-cursor").join("shared");
        let portable = home.join(".agents").join("skills").join("shared");
        let library = root.join("library");
        let backup = library.join("shared");
        for directory in [&legacy, &portable, &backup] {
            fs::create_dir_all(directory).expect("Skill directory should be created");
            fs::write(directory.join("SKILL.md"), "---\nname: shared\n---\n")
                .expect("Skill should be written");
        }
        write_managed_origin(&backup, &legacy).expect("legacy backup should be marked");
        let origin = normalized_identity_path(&legacy);
        assert!(shadowed_cursor_portable_skill(&origin, &home));

        let mut warnings = Vec::new();
        archive_shadowed_cursor_library_skills(&library, &home, &mut warnings)
            .expect("shadowed backup should be archived");
        assert!(warnings.is_empty());
        assert!(!backup.exists());
        assert!(library.join(MANAGED_SKILL_HISTORY_DIR).is_dir());

        let unique = home
            .join(".cursor")
            .join("skills-cursor")
            .join("cursor-only");
        fs::create_dir_all(&unique).expect("unique Cursor Skill should be created");
        fs::write(unique.join("SKILL.md"), "---\nname: cursor-only\n---\n")
            .expect("unique Cursor Skill should be written");
        assert!(!shadowed_cursor_portable_skill(
            &normalized_identity_path(&unique),
            &home
        ));
        fs::remove_dir_all(root).expect("temporary directory should be removed");
    }

    #[test]
    fn claude_override_preserves_unrelated_user_settings() {
        let root = temporary_directory("claude-disable-config");
        let settings = root.join(".claude").join("settings.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        fs::write(
            &settings,
            r#"{"theme":"dark","skillOverrides":{"other":"on"}}"#,
        )
        .unwrap();
        let previous = set_claude_skill_override(&root, "demo", true).unwrap();
        assert!(previous.is_none());
        let disabled: serde_json::Value =
            serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
        assert_eq!(disabled["theme"], "dark");
        assert_eq!(disabled["skillOverrides"]["other"], "on");
        assert_eq!(disabled["skillOverrides"]["demo"], "off");
        set_claude_skill_override(&root, "demo", false).unwrap();
        let enabled: serde_json::Value =
            serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
        assert!(enabled["skillOverrides"].get("demo").is_none());
        assert_eq!(enabled["skillOverrides"]["other"], "on");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn codex_skill_toggle_updates_one_entry_without_overwriting_config() {
        let root = temporary_directory("codex-disable-config");
        let config = root.join(".codex").join("config.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(
            &config,
            "model = \"test-model\"\n\n[[skills.config]]\npath = \"C:/other/SKILL.md\"\nenabled = false\n",
        )
        .unwrap();
        set_codex_skill_enabled(&root, "C:/demo/SKILL.md", false).unwrap();
        set_codex_skill_enabled(&root, "C:/demo/SKILL.md", true).unwrap();
        let document = fs::read_to_string(&config)
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        assert_eq!(document["model"].as_str(), Some("test-model"));
        let entries = document["skills"]["config"].as_array_of_tables().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries.get(0).unwrap()["enabled"].as_bool(), Some(false));
        assert_eq!(
            entries.get(1).unwrap()["path"].as_str(),
            Some("C:/demo/SKILL.md")
        );
        assert_eq!(entries.get(1).unwrap()["enabled"].as_bool(), Some(true));
        fs::remove_dir_all(root).unwrap();
    }
}
