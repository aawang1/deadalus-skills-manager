use crate::vectorization::SearchResult;
use serde::Serialize;
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub const LOG_FILE_NAME: &str = "agent-task-log.jsonl";
static LOG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLogSkillRef {
    pub skill_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLogRetrievalSettings {
    pub parent_pool_min: usize,
    pub parent_pool_max: usize,
    pub parent_min_score: f32,
    pub final_min_score: f32,
    pub score_gap: f32,
    pub max_final_results: usize,
    pub category_fit_weight: f32,
    pub vector_fit_weight: f32,
    pub global_rescue_threshold: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskLog {
    pub schema_version: u32,
    pub task_id: String,
    pub timestamp_ms: u128,
    pub status: String,
    pub phase: String,
    pub request: String,
    pub view_id: String,
    pub graph_version: Option<String>,
    pub eligible_skills: Vec<AgentLogSkillRef>,
    pub analysis_provider: Option<String>,
    pub analysis_model: Option<String>,
    pub analysis_summary: Option<String>,
    pub search_query: Option<String>,
    pub requirement_analysis: Option<Value>,
    pub category_matches: Option<Value>,
    pub skill_matches: Option<Value>,
    pub result_sources: Option<Value>,
    pub embedding_profile_id: Option<String>,
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    pub retrieval_settings: Option<AgentLogRetrievalSettings>,
    pub result_names: Vec<AgentLogSkillRef>,
    pub results: Vec<SearchResult>,
    pub error: Option<String>,
}

impl AgentTaskLog {
    pub fn new(request: String, view_id: String) -> Self {
        Self {
            schema_version: 2,
            task_id: Uuid::new_v4().to_string(),
            timestamp_ms: now_ms(),
            status: "started".to_string(),
            phase: "validate_request".to_string(),
            request,
            view_id,
            graph_version: None,
            eligible_skills: Vec::new(),
            analysis_provider: None,
            analysis_model: None,
            analysis_summary: None,
            search_query: None,
            requirement_analysis: None,
            category_matches: None,
            skill_matches: None,
            result_sources: None,
            embedding_profile_id: None,
            embedding_provider: None,
            embedding_model: None,
            retrieval_settings: None,
            result_names: Vec::new(),
            results: Vec::new(),
            error: None,
        }
    }

    pub fn finish(&mut self, result: &Result<(), String>) {
        self.timestamp_ms = now_ms();
        match result {
            Ok(()) => {
                self.status = "succeeded".to_string();
                self.phase = "completed".to_string();
            }
            Err(error) => {
                self.status = "failed".to_string();
                self.error = Some(error.clone());
            }
        }
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

pub fn append_task_log(app_data_dir: &Path, entry: &AgentTaskLog) -> io::Result<()> {
    let lock = LOG_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock
        .lock()
        .map_err(|_| io::Error::other("task log lock poisoned"))?;
    fs::create_dir_all(app_data_dir)?;
    let line = serde_json::to_vec(entry).map_err(io::Error::other)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(app_data_dir.join(LOG_FILE_NAME))?;
    file.write_all(&line)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_started_and_finished_records_without_losing_request_context() {
        let dir = std::env::temp_dir().join(format!("deadalus-task-log-{}", Uuid::new_v4()));
        let mut entry = AgentTaskLog::new("诗词网站".to_string(), "all".to_string());
        append_task_log(&dir, &entry).unwrap();
        entry.phase = "analysis".to_string();
        entry.finish(&Err("analysis failed".to_string()));
        append_task_log(&dir, &entry).unwrap();
        let contents = fs::read_to_string(dir.join(LOG_FILE_NAME)).unwrap();
        let lines = contents
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["status"], "started");
        assert_eq!(lines[1]["status"], "failed");
        assert_eq!(lines[1]["request"], "诗词网站");
        assert_eq!(lines[1]["error"], "analysis failed");
        fs::remove_dir_all(dir).unwrap();
    }
}
