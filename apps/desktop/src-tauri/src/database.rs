use crate::vectorization::{
    EmbeddingJob, EmbeddingProfile, GeneratedInput, JobStatus, ProfileStatus, RelationshipState,
    RelationshipType, SkillRelationship, StoredVector, ValidationState, VectorType,
};
use crate::{
    adaptive::{AdaptivePolicyState, AdaptiveSignals},
    application::{
        AnalysisComparison, ConflictResolutionResult, LlmAnalysisResult, RuleAnalysisResult,
    },
    pipeline::{IndexDiff, IndexSyncStatus},
    validation::SavedAnalysisRecord,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard};
use thiserror::Error;

const CURRENT_SCHEMA_VERSION: i64 = 4;

type RawProfile = (
    String,
    String,
    String,
    String,
    u32,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    bool,
    i64,
    Option<i64>,
    Option<String>,
);

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("database lock is poisoned")]
    Poisoned,
    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },
    #[error("invalid persisted value for {field}: {value}")]
    InvalidValue { field: &'static str, value: String },
}

pub type DatabaseResult<T> = Result<T, DatabaseError>;

pub struct Database {
    connection: Mutex<Connection>,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> DatabaseResult<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                DatabaseError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
            })?;
        }
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> DatabaseResult<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(mut connection: Connection) -> DatabaseResult<Self> {
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn connection(&self) -> DatabaseResult<MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| DatabaseError::Poisoned)
    }

    pub fn schema_version(&self) -> DatabaseResult<i64> {
        self.connection()?
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn upsert_profile(&self, profile: &EmbeddingProfile) -> DatabaseResult<()> {
        let connection = self.connection()?;
        connection.execute(
            r#"
            INSERT INTO embedding_profiles (
                profile_id, provider, model, model_version, dimensions,
                input_schema_version, chunk_policy_version, tokenizer,
                credential_id, status, is_active, created_at, activated_at, error
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            ON CONFLICT(profile_id) DO UPDATE SET
                provider = excluded.provider,
                model = excluded.model,
                model_version = excluded.model_version,
                dimensions = excluded.dimensions,
                input_schema_version = excluded.input_schema_version,
                chunk_policy_version = excluded.chunk_policy_version,
                tokenizer = excluded.tokenizer,
                credential_id = excluded.credential_id,
                status = excluded.status,
                error = excluded.error
            "#,
            params![
                profile.profile_id,
                profile.provider,
                profile.model,
                profile.model_version,
                profile.dimensions,
                profile.input_schema_version,
                profile.chunk_policy_version,
                profile.tokenizer,
                profile.credential_id,
                profile.status.as_str(),
                profile.is_active,
                profile.created_at,
                profile.activated_at,
                profile.error,
            ],
        )?;
        Ok(())
    }

    pub fn activate_profile_atomic(
        &self,
        profile_id: &str,
        activated_at: i64,
    ) -> DatabaseResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let target_status: Option<String> = transaction
            .query_row(
                "SELECT status FROM embedding_profiles WHERE profile_id = ?1",
                [profile_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(target_status) = target_status else {
            return Err(DatabaseError::NotFound {
                entity: "embedding profile",
                id: profile_id.to_string(),
            });
        };
        if !matches!(target_status.as_str(), "ready" | "active") {
            return Err(DatabaseError::InvalidValue {
                field: "embedding_profiles.status",
                value: target_status,
            });
        }
        transaction.execute(
            r#"
            UPDATE embedding_profiles
            SET is_active = 0,
                status = CASE WHEN status = 'active' THEN 'ready' ELSE status END
            WHERE is_active = 1
            "#,
            [],
        )?;
        transaction.execute(
            r#"
            UPDATE embedding_profiles
            SET is_active = 1, status = 'active', activated_at = ?2, error = NULL
            WHERE profile_id = ?1
            "#,
            params![profile_id, activated_at],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn activate_profile_and_embedding_credential_atomic(
        &self,
        profile_id: &str,
        credential_id: &str,
        provider: &str,
        activated_at: i64,
    ) -> DatabaseResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let target: Option<(String, String, Option<String>)> = transaction
            .query_row(
                "SELECT status, provider, credential_id FROM embedding_profiles WHERE profile_id = ?1",
                [profile_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((status, target_provider, target_credential)) = target else {
            return Err(DatabaseError::NotFound {
                entity: "embedding profile",
                id: profile_id.to_string(),
            });
        };
        if status != "ready"
            || target_provider != provider
            || target_credential.as_deref() != Some(credential_id)
        {
            return Err(DatabaseError::InvalidValue {
                field: "embedding_profiles.activation_target",
                value: format!("{status}:{target_provider}:{target_credential:?}"),
            });
        }
        transaction.execute(
            r#"
            UPDATE embedding_profiles
            SET is_active = 0,
                status = CASE WHEN status = 'active' THEN 'ready' ELSE status END
            WHERE is_active = 1
            "#,
            [],
        )?;
        transaction.execute(
            r#"
            UPDATE embedding_profiles
            SET is_active = 1, status = 'active', activated_at = ?2, error = NULL
            WHERE profile_id = ?1
            "#,
            params![profile_id, activated_at],
        )?;
        transaction.execute(
            r#"
            UPDATE credential_purpose_bindings
            SET is_active = 0, updated_at = ?1
            WHERE purpose = 'embedding'
            "#,
            [activated_at],
        )?;
        transaction.execute(
            r#"
            INSERT INTO credential_purpose_bindings (
                credential_id, purpose, provider, is_active, updated_at
            ) VALUES (?1, 'embedding', ?2, 1, ?3)
            ON CONFLICT(credential_id, purpose) DO UPDATE SET
                provider = excluded.provider,
                is_active = 1,
                updated_at = excluded.updated_at
            "#,
            params![credential_id, provider, activated_at],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn update_profile_credential(
        &self,
        profile_id: &str,
        credential_id: &str,
    ) -> DatabaseResult<()> {
        let updated = self.connection()?.execute(
            "UPDATE embedding_profiles SET credential_id = ?2, error = NULL WHERE profile_id = ?1",
            params![profile_id, credential_id],
        )?;
        if updated == 0 {
            return Err(DatabaseError::NotFound {
                entity: "embedding profile",
                id: profile_id.to_string(),
            });
        }
        Ok(())
    }

    pub fn rotate_active_profile_credential_atomic(
        &self,
        profile_id: &str,
        credential_id: &str,
        provider: &str,
        updated_at: i64,
    ) -> DatabaseResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let active_provider: Option<String> = transaction
            .query_row(
                r#"
                SELECT provider FROM embedding_profiles
                WHERE profile_id = ?1 AND is_active = 1 AND status = 'active'
                "#,
                [profile_id],
                |row| row.get(0),
            )
            .optional()?;
        if active_provider.as_deref() != Some(provider) {
            return Err(DatabaseError::InvalidValue {
                field: "embedding_profiles.active_provider",
                value: active_provider.unwrap_or_default(),
            });
        }
        transaction.execute(
            "UPDATE embedding_profiles SET credential_id = ?2, error = NULL WHERE profile_id = ?1",
            params![profile_id, credential_id],
        )?;
        transaction.execute(
            r#"
            UPDATE credential_purpose_bindings
            SET is_active = 0, updated_at = ?1
            WHERE purpose = 'embedding'
            "#,
            [updated_at],
        )?;
        transaction.execute(
            r#"
            INSERT INTO credential_purpose_bindings (
                credential_id, purpose, provider, is_active, updated_at
            ) VALUES (?1, 'embedding', ?2, 1, ?3)
            ON CONFLICT(credential_id, purpose) DO UPDATE SET
                provider = excluded.provider,
                is_active = 1,
                updated_at = excluded.updated_at
            "#,
            params![credential_id, provider, updated_at],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn active_profile(&self) -> DatabaseResult<Option<EmbeddingProfile>> {
        let connection = self.connection()?;
        let raw = connection
            .query_row(
                r#"
                SELECT profile_id, provider, model, model_version, dimensions,
                       input_schema_version, chunk_policy_version, tokenizer,
                       credential_id, status, is_active, created_at, activated_at, error
                FROM embedding_profiles
                WHERE is_active = 1
                "#,
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, u32>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, bool>(10)?,
                        row.get::<_, i64>(11)?,
                        row.get::<_, Option<i64>>(12)?,
                        row.get::<_, Option<String>>(13)?,
                    ))
                },
            )
            .optional()?;
        raw.map(profile_from_raw).transpose()
    }

    pub fn profile(&self, profile_id: &str) -> DatabaseResult<Option<EmbeddingProfile>> {
        let connection = self.connection()?;
        let raw = connection
            .query_row(
                r#"
                SELECT profile_id, provider, model, model_version, dimensions,
                       input_schema_version, chunk_policy_version, tokenizer,
                       credential_id, status, is_active, created_at, activated_at, error
                FROM embedding_profiles
                WHERE profile_id = ?1
                "#,
                [profile_id],
                raw_profile_from_row,
            )
            .optional()?;
        raw.map(profile_from_raw).transpose()
    }

    pub fn list_profiles(&self) -> DatabaseResult<Vec<EmbeddingProfile>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            r#"
            SELECT profile_id, provider, model, model_version, dimensions,
                   input_schema_version, chunk_policy_version, tokenizer,
                   credential_id, status, is_active, created_at, activated_at, error
            FROM embedding_profiles
            ORDER BY is_active DESC, created_at DESC, profile_id
            "#,
        )?;
        let rows = statement.query_map([], raw_profile_from_row)?;
        let mut profiles = Vec::new();
        for row in rows {
            profiles.push(profile_from_raw(row?)?);
        }
        Ok(profiles)
    }

    pub fn activate_credential_binding(
        &self,
        credential_id: &str,
        purpose: &str,
        provider: &str,
        updated_at: i64,
    ) -> DatabaseResult<()> {
        if !matches!(purpose, "agent" | "embedding") {
            return Err(DatabaseError::InvalidValue {
                field: "credential_purpose_bindings.purpose",
                value: purpose.to_string(),
            });
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE credential_purpose_bindings SET is_active = 0, updated_at = ?2 WHERE purpose = ?1",
            params![purpose, updated_at],
        )?;
        transaction.execute(
            r#"
            INSERT INTO credential_purpose_bindings (
                credential_id, purpose, provider, is_active, updated_at
            ) VALUES (?1, ?2, ?3, 1, ?4)
            ON CONFLICT(credential_id, purpose) DO UPDATE SET
                provider = excluded.provider,
                is_active = 1,
                updated_at = excluded.updated_at
            "#,
            params![credential_id, purpose, provider, updated_at],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn active_credential_binding(
        &self,
        purpose: &str,
    ) -> DatabaseResult<Option<CredentialPurposeBinding>> {
        self.connection()?
            .query_row(
                r#"
                SELECT credential_id, purpose, provider, is_active, updated_at
                FROM credential_purpose_bindings
                WHERE purpose = ?1 AND is_active = 1
                "#,
                [purpose],
                |row| {
                    Ok(CredentialPurposeBinding {
                        credential_id: row.get(0)?,
                        purpose: row.get(1)?,
                        provider: row.get(2)?,
                        is_active: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn detach_credential(&self, credential_id: &str) -> DatabaseResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM credential_purpose_bindings WHERE credential_id = ?1",
            [credential_id],
        )?;
        transaction.execute(
            r#"
            UPDATE embedding_profiles
            SET credential_id = NULL,
                error = CASE
                    WHEN status IN ('active', 'ready') THEN 'credential_deleted'
                    ELSE error
                END
            WHERE credential_id = ?1
            "#,
            [credential_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_relationship(&self, relationship: &SkillRelationship) -> DatabaseResult<()> {
        let evidence = serde_json::to_string(&relationship.evidence)?;
        self.connection()?.execute(
            r#"
            INSERT INTO skill_relations (
                relation_id, source_skill_id, target_skill_id, relationship_type,
                vector_type, source_profile_id, target_profile_id, score, state,
                evidence_json, created_at, validated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            ON CONFLICT(relation_id) DO UPDATE SET
                score = excluded.score,
                state = excluded.state,
                evidence_json = excluded.evidence_json,
                target_profile_id = excluded.target_profile_id,
                validated_at = excluded.validated_at
            "#,
            params![
                relationship.relation_id,
                relationship.source_skill_id,
                relationship.target_skill_id,
                relationship.relationship_type.as_str(),
                relationship.vector_type.map(VectorType::as_str),
                relationship.source_profile_id,
                relationship.target_profile_id,
                relationship.score,
                relationship.state.as_str(),
                evidence,
                relationship.created_at,
                relationship.validated_at,
            ],
        )?;
        Ok(())
    }

    pub fn relationships_for_skill(
        &self,
        skill_id: &str,
    ) -> DatabaseResult<Vec<SkillRelationship>> {
        self.relationships_for_skill_profile(skill_id, None)
    }

    pub fn relationships_for_skill_profile(
        &self,
        skill_id: &str,
        profile_id: Option<&str>,
    ) -> DatabaseResult<Vec<SkillRelationship>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            r#"
            SELECT relation_id, source_skill_id, target_skill_id, relationship_type,
                   vector_type, source_profile_id, target_profile_id, score, state,
                   evidence_json, created_at, validated_at
            FROM skill_relations
            WHERE (source_skill_id = ?1 OR target_skill_id = ?1)
              AND (
                    ?2 IS NULL
                    OR source_profile_id = ?2
                    OR target_profile_id = ?2
                    OR (
                        state IN ('carried_fact', 'human_confirmed', 'human_rejected')
                        AND source_profile_id IS NULL
                        AND target_profile_id IS NULL
                    )
              )
            ORDER BY created_at, relation_id
            "#,
        )?;
        let rows = statement.query_map(params![skill_id, profile_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<f64>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, Option<i64>>(11)?,
            ))
        })?;
        let mut relationships = Vec::new();
        for row in rows {
            let (
                relation_id,
                source_skill_id,
                target_skill_id,
                relationship_type,
                vector_type,
                source_profile_id,
                target_profile_id,
                score,
                state,
                evidence,
                created_at,
                validated_at,
            ) = row?;
            relationships.push(SkillRelationship {
                relation_id,
                source_skill_id,
                target_skill_id,
                relationship_type: parse_relationship_type(&relationship_type)?,
                vector_type: vector_type
                    .map(|value| parse_value("skill_relations.vector_type", &value))
                    .transpose()?,
                source_profile_id,
                target_profile_id,
                score,
                state: parse_relationship_state(&state)?,
                evidence: serde_json::from_str(&evidence)?,
                created_at,
                validated_at,
            });
        }
        Ok(relationships)
    }

    pub fn relationships_for_profile(
        &self,
        profile_id: &str,
    ) -> DatabaseResult<Vec<SkillRelationship>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            r#"
            SELECT relation_id, source_skill_id, target_skill_id, relationship_type,
                   vector_type, source_profile_id, target_profile_id, score, state,
                   evidence_json, created_at, validated_at
            FROM skill_relations
            WHERE source_profile_id = ?1
               OR target_profile_id = ?1
               OR (
                    state IN ('carried_fact', 'human_confirmed', 'human_rejected')
                    AND source_profile_id IS NULL
                    AND target_profile_id IS NULL
               )
            ORDER BY created_at, relation_id
            "#,
        )?;
        let rows = statement.query_map([profile_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<f64>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, Option<i64>>(11)?,
            ))
        })?;
        let mut relationships = Vec::new();
        for row in rows {
            let row = row?;
            relationships.push(SkillRelationship {
                relation_id: row.0,
                source_skill_id: row.1,
                target_skill_id: row.2,
                relationship_type: parse_relationship_type(&row.3)?,
                vector_type: row
                    .4
                    .map(|value| parse_value("skill_relations.vector_type", &value))
                    .transpose()?,
                source_profile_id: row.5,
                target_profile_id: row.6,
                score: row.7,
                state: parse_relationship_state(&row.8)?,
                evidence: serde_json::from_str(&row.9)?,
                created_at: row.10,
                validated_at: row.11,
            });
        }
        Ok(relationships)
    }

    pub fn append_adaptive_history(&self, record: &AdaptiveHistoryRecord) -> DatabaseResult<()> {
        self.connection()?.execute(
            r#"
            INSERT INTO adaptive_history (
                history_id, profile_id, parameter_version, proposal_json,
                evidence_json, decision, created_at, applied_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                record.history_id,
                record.profile_id,
                record.parameter_version,
                serde_json::to_string(&record.proposal)?,
                serde_json::to_string(&record.evidence)?,
                record.decision,
                record.created_at,
                record.applied_at,
            ],
        )?;
        Ok(())
    }

    pub fn adaptive_history(&self, profile_id: &str) -> DatabaseResult<Vec<AdaptiveHistoryRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            r#"
            SELECT history_id, profile_id, parameter_version, proposal_json,
                   evidence_json, decision, created_at, applied_at
            FROM adaptive_history
            WHERE profile_id = ?1
            ORDER BY created_at, history_id
            "#,
        )?;
        let rows = statement.query_map([profile_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, Option<i64>>(7)?,
            ))
        })?;
        let mut history = Vec::new();
        for row in rows {
            let (
                history_id,
                profile_id,
                parameter_version,
                proposal,
                evidence,
                decision,
                created_at,
                applied_at,
            ) = row?;
            history.push(AdaptiveHistoryRecord {
                history_id,
                profile_id,
                parameter_version,
                proposal: serde_json::from_str(&proposal)?,
                evidence: serde_json::from_str(&evidence)?,
                decision,
                created_at,
                applied_at,
            });
        }
        Ok(history)
    }

    pub fn adaptive_history_record(
        &self,
        history_id: &str,
    ) -> DatabaseResult<Option<AdaptiveHistoryRecord>> {
        self.connection()?
            .query_row(
                r#"
                SELECT history_id, profile_id, parameter_version, proposal_json,
                       evidence_json, decision, created_at, applied_at
                FROM adaptive_history WHERE history_id = ?1
                "#,
                [history_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                    ))
                },
            )
            .optional()?
            .map(|row| {
                Ok(AdaptiveHistoryRecord {
                    history_id: row.0,
                    profile_id: row.1,
                    parameter_version: row.2,
                    proposal: serde_json::from_str(&row.3)?,
                    evidence: serde_json::from_str(&row.4)?,
                    decision: row.5,
                    created_at: row.6,
                    applied_at: row.7,
                })
            })
            .transpose()
    }

    pub fn update_adaptive_history_decision(
        &self,
        history_id: &str,
        expected_decisions: &[&str],
        decision: &str,
        applied_at: Option<i64>,
    ) -> DatabaseResult<bool> {
        let connection = self.connection()?;
        let current: Option<String> = connection
            .query_row(
                "SELECT decision FROM adaptive_history WHERE history_id = ?1",
                [history_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(current) = current else {
            return Ok(false);
        };
        if !expected_decisions.contains(&current.as_str()) {
            return Err(DatabaseError::InvalidValue {
                field: "adaptive_history.decision",
                value: current,
            });
        }
        connection.execute(
            "UPDATE adaptive_history SET decision = ?2, applied_at = ?3 WHERE history_id = ?1",
            params![history_id, decision, applied_at],
        )?;
        Ok(true)
    }

    pub fn adaptive_policy(&self, profile_id: &str) -> DatabaseResult<Option<AdaptivePolicyState>> {
        self.connection()?
            .query_row(
                r#"
                SELECT profile_id, h, m, u, l, local_update_enabled, policy_version,
                       limit_source, limit_source_version, applied_history_id,
                       effective_after, updated_at
                FROM adaptive_policy_state WHERE profile_id = ?1
                "#,
                [profile_id],
                |row| {
                    Ok(AdaptivePolicyState {
                        profile_id: row.get(0)?,
                        h: row.get::<_, i64>(1)?.max(0) as u64,
                        m: row.get::<_, i64>(2)?.max(0) as u64,
                        u: row.get::<_, i64>(3)?.max(0) as u64,
                        l: row.get::<_, i64>(4)?.max(0) as u64,
                        local_update_enabled: row.get(5)?,
                        policy_version: row.get(6)?,
                        limit_source: row.get(7)?,
                        limit_source_version: row.get(8)?,
                        applied_history_id: row.get(9)?,
                        effective_after: row.get(10)?,
                        updated_at: row.get(11)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn save_adaptive_policy(&self, policy: &AdaptivePolicyState) -> DatabaseResult<()> {
        policy
            .validate()
            .map_err(|value| DatabaseError::InvalidValue {
                field: "adaptive_policy_state",
                value,
            })?;
        self.connection()?.execute(
            r#"
            INSERT INTO adaptive_policy_state (
                profile_id, h, m, u, l, local_update_enabled, policy_version,
                limit_source, limit_source_version, applied_history_id,
                effective_after, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            ON CONFLICT(profile_id) DO UPDATE SET
                h = excluded.h,
                m = excluded.m,
                u = excluded.u,
                l = excluded.l,
                local_update_enabled = excluded.local_update_enabled,
                policy_version = excluded.policy_version,
                limit_source = excluded.limit_source,
                limit_source_version = excluded.limit_source_version,
                applied_history_id = excluded.applied_history_id,
                effective_after = excluded.effective_after,
                updated_at = excluded.updated_at
            "#,
            params![
                policy.profile_id,
                policy.h as i64,
                policy.m as i64,
                policy.u as i64,
                policy.l as i64,
                policy.local_update_enabled,
                policy.policy_version,
                policy.limit_source,
                policy.limit_source_version,
                policy.applied_history_id,
                policy.effective_after,
                policy.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn adaptive_signals(&self, profile_id: &str) -> DatabaseResult<AdaptiveSignals> {
        let connection = self.connection()?;
        let mut lengths = Vec::new();
        {
            let mut statement = connection.prepare(
                r#"
                SELECT DISTINCT MAX(1, LENGTH(i.input_text) / 4)
                FROM embedding_inputs i
                JOIN embeddings e ON e.input_hash = i.input_hash
                WHERE e.profile_id = ?1
                "#,
            )?;
            lengths.extend(
                statement
                    .query_map([profile_id], |row| row.get::<_, i64>(0))?
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .map(|value| value.max(1) as u64),
            );
        }
        {
            let mut statement = connection.prepare(
                r#"
                SELECT DISTINCT c.token_estimate
                FROM embedding_chunks c
                JOIN embeddings e ON e.chunk_id = c.chunk_id
                WHERE e.profile_id = ?1
                "#,
            )?;
            lengths.extend(
                statement
                    .query_map([profile_id], |row| row.get::<_, i64>(0))?
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .map(|value| value.max(1) as u64),
            );
        }
        let parent_count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM embeddings WHERE profile_id = ?1 AND level = 'parent'",
            [profile_id],
            |row| row.get(0),
        )?;
        let chunk_count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM embeddings WHERE profile_id = ?1 AND level = 'chunk'",
            [profile_id],
            |row| row.get(0),
        )?;
        let (successful_calls, failed_calls, actual_tokens): (i64, i64, i64) = connection
            .query_row(
                r#"
                SELECT
                    SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END),
                    COALESCE(SUM(actual_tokens), 0)
                FROM embedding_jobs WHERE profile_id = ?1
                "#,
                [profile_id],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?.unwrap_or_default(),
                        row.get::<_, Option<i64>>(1)?.unwrap_or_default(),
                        row.get(2)?,
                    ))
                },
            )?;
        let latest_quality_metrics = connection
            .query_row(
                r#"
                SELECT metrics_json FROM validation_runs
                WHERE profile_id = ?1 AND status = 'completed'
                ORDER BY completed_at DESC, run_id DESC LIMIT 1
                "#,
                [profile_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|value| serde_json::from_str(&value))
            .transpose()?;
        Ok(AdaptiveSignals {
            token_lengths: lengths,
            parent_count: parent_count.max(0) as u64,
            chunk_count: chunk_count.max(0) as u64,
            successful_calls: successful_calls.max(0) as u64,
            failed_calls: failed_calls.max(0) as u64,
            actual_tokens: actual_tokens.max(0) as u64,
            latest_quality_metrics,
        })
    }

    pub fn save_validation_sample(&self, sample: &ValidationSampleRecord) -> DatabaseResult<()> {
        self.connection()?.execute(
            r#"
            INSERT INTO validation_samples (
                sample_id, dataset_id, dataset_schema_version, dataset_content_version,
                split, skill_id, chunk_id, query_text, query_language, task_type,
                label, state, evidence_json, confidence, profile_id, input_hash,
                created_at, reviewed_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18
            )
            ON CONFLICT(sample_id) DO UPDATE SET
                dataset_content_version = excluded.dataset_content_version,
                label = excluded.label,
                state = excluded.state,
                evidence_json = excluded.evidence_json,
                confidence = excluded.confidence,
                reviewed_at = excluded.reviewed_at
            "#,
            params![
                sample.sample_id,
                sample.dataset_id,
                sample.dataset_schema_version,
                sample.dataset_content_version,
                sample.split,
                sample.skill_id,
                sample.chunk_id,
                sample.query_text,
                sample.query_language,
                sample.task_type,
                sample.label,
                sample.state.as_str(),
                serde_json::to_string(&sample.evidence)?,
                sample.confidence,
                sample.profile_id,
                sample.input_hash,
                sample.created_at,
                sample.reviewed_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_validation_samples(
        &self,
        profile_id: Option<&str>,
        split: Option<&str>,
    ) -> DatabaseResult<Vec<ValidationSampleRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            r#"
            SELECT sample_id, dataset_id, dataset_schema_version, dataset_content_version,
                   split, skill_id, chunk_id, query_text, query_language, task_type,
                   label, state, evidence_json, confidence, profile_id, input_hash,
                   created_at, reviewed_at
            FROM validation_samples
            WHERE (?1 IS NULL OR profile_id IS NULL OR profile_id = ?1)
              AND (?2 IS NULL OR split = ?2)
            ORDER BY created_at DESC, sample_id
            "#,
        )?;
        let rows = statement.query_map(params![profile_id, split], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, Option<f64>>(13)?,
                row.get::<_, Option<String>>(14)?,
                row.get::<_, String>(15)?,
                row.get::<_, i64>(16)?,
                row.get::<_, Option<i64>>(17)?,
            ))
        })?;
        let mut samples = Vec::new();
        for row in rows {
            let row = row?;
            samples.push(ValidationSampleRecord {
                sample_id: row.0,
                dataset_id: row.1,
                dataset_schema_version: row.2,
                dataset_content_version: row.3,
                split: row.4,
                skill_id: row.5,
                chunk_id: row.6,
                query_text: row.7,
                query_language: row.8,
                task_type: row.9,
                label: row.10,
                state: parse_value("validation_samples.state", &row.11)?,
                evidence: serde_json::from_str(&row.12)?,
                confidence: row.13,
                profile_id: row.14,
                input_hash: row.15,
                created_at: row.16,
                reviewed_at: row.17,
            });
        }
        Ok(samples)
    }

    pub fn save_validation_run(&self, run: &ValidationRunRecord) -> DatabaseResult<()> {
        self.connection()?.execute(
            r#"
            INSERT INTO validation_runs (
                run_id, dataset_id, dataset_content_version, profile_id, status,
                metrics_json, started_at, completed_at, error
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(run_id) DO UPDATE SET
                status = excluded.status,
                metrics_json = excluded.metrics_json,
                completed_at = excluded.completed_at,
                error = excluded.error
            "#,
            params![
                run.run_id,
                run.dataset_id,
                run.dataset_content_version,
                run.profile_id,
                run.status,
                serde_json::to_string(&run.metrics)?,
                run.started_at,
                run.completed_at,
                run.error,
            ],
        )?;
        Ok(())
    }

    pub fn list_validation_runs(
        &self,
        profile_id: Option<&str>,
    ) -> DatabaseResult<Vec<ValidationRunRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            r#"
            SELECT run_id, dataset_id, dataset_content_version, profile_id, status,
                   metrics_json, started_at, completed_at, error
            FROM validation_runs
            WHERE ?1 IS NULL OR profile_id = ?1
            ORDER BY started_at DESC, run_id
            "#,
        )?;
        let rows = statement.query_map([profile_id], |row| {
            Ok(ValidationRunRecord {
                run_id: row.get(0)?,
                dataset_id: row.get(1)?,
                dataset_content_version: row.get(2)?,
                profile_id: row.get(3)?,
                status: row.get(4)?,
                metrics: serde_json::from_str::<serde_json::Value>(&row.get::<_, String>(5)?)
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            5,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                started_at: row.get(6)?,
                completed_at: row.get(7)?,
                error: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn saved_analysis_records(&self) -> DatabaseResult<Vec<SavedAnalysisRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            r#"
            SELECT skills.skill_id,
                   (
                       SELECT r.result_json
                       FROM analysis_rule_results r
                       WHERE r.skill_id = skills.skill_id
                       ORDER BY r.created_at DESC, r.result_id DESC
                       LIMIT 1
                   ),
                   (
                       SELECT l.result_json
                       FROM analysis_llm_results l
                       WHERE l.skill_id = skills.skill_id
                       ORDER BY l.created_at DESC, l.result_id DESC
                       LIMIT 1
                   ),
                   (
                       SELECT c.status
                       FROM analysis_comparisons c
                       JOIN analysis_llm_results l ON l.result_id = c.llm_result_id
                       WHERE l.skill_id = skills.skill_id
                       ORDER BY c.created_at DESC, c.comparison_id DESC
                       LIMIT 1
                   )
            FROM (
                SELECT skill_id FROM analysis_rule_results
                UNION
                SELECT skill_id FROM analysis_llm_results
            ) skills
            ORDER BY skills.skill_id
            "#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut latest = BTreeMap::new();
        for row in rows {
            let (skill_id, rule_result, llm_result, comparison_status) = row?;
            latest.insert(
                skill_id.clone(),
                SavedAnalysisRecord {
                    skill_id,
                    rule_result: rule_result
                        .map(|result| serde_json::from_str(&result))
                        .transpose()?,
                    comparison_status,
                    llm_result: llm_result
                        .map(|result| serde_json::from_str(&result))
                        .transpose()?,
                },
            );
        }
        Ok(latest.into_values().collect())
    }

    pub fn save_feedback_event(&self, event: &FeedbackEventRecord) -> DatabaseResult<()> {
        self.connection()?.execute(
            r#"
            INSERT INTO feedback_events (
                feedback_id, dataset_id, skill_id, query_id, other_skill_id,
                relation_type, action, strength, before_state_json, after_state_json,
                profile_id, source_evidence_json, confirmed_at, reverted_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "#,
            params![
                event.feedback_id,
                event.dataset_id,
                event.skill_id,
                event.query_id,
                event.other_skill_id,
                event.relation_type,
                event.action,
                event.strength,
                serde_json::to_string(&event.before_state)?,
                serde_json::to_string(&event.after_state)?,
                event.profile_id,
                serde_json::to_string(&event.source_evidence)?,
                event.confirmed_at,
                event.reverted_at,
            ],
        )?;
        Ok(())
    }

    pub fn record_feedback_bundle(
        &self,
        event: &FeedbackEventRecord,
        relationship: Option<&SkillRelationship>,
        sample: Option<&ValidationSampleRecord>,
    ) -> DatabaseResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            r#"
            INSERT INTO feedback_events (
                feedback_id, dataset_id, skill_id, query_id, other_skill_id,
                relation_type, action, strength, before_state_json, after_state_json,
                profile_id, source_evidence_json, confirmed_at, reverted_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "#,
            params![
                event.feedback_id,
                event.dataset_id,
                event.skill_id,
                event.query_id,
                event.other_skill_id,
                event.relation_type,
                event.action,
                event.strength,
                serde_json::to_string(&event.before_state)?,
                serde_json::to_string(&event.after_state)?,
                event.profile_id,
                serde_json::to_string(&event.source_evidence)?,
                event.confirmed_at,
                event.reverted_at,
            ],
        )?;
        if let Some(relationship) = relationship {
            transaction.execute(
                r#"
                INSERT INTO skill_relations (
                    relation_id, source_skill_id, target_skill_id, relationship_type,
                    vector_type, source_profile_id, target_profile_id, score, state,
                    evidence_json, created_at, validated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                "#,
                params![
                    relationship.relation_id,
                    relationship.source_skill_id,
                    relationship.target_skill_id,
                    relationship.relationship_type.as_str(),
                    relationship.vector_type.map(VectorType::as_str),
                    relationship.source_profile_id,
                    relationship.target_profile_id,
                    relationship.score,
                    relationship.state.as_str(),
                    serde_json::to_string(&relationship.evidence)?,
                    relationship.created_at,
                    relationship.validated_at,
                ],
            )?;
        }
        if let Some(sample) = sample {
            insert_validation_sample(&transaction, sample)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn feedback_event(&self, feedback_id: &str) -> DatabaseResult<Option<FeedbackEventRecord>> {
        self.connection()?
            .query_row(
                r#"
                SELECT feedback_id, dataset_id, skill_id, query_id, other_skill_id,
                       relation_type, action, strength, before_state_json, after_state_json,
                       profile_id, source_evidence_json, confirmed_at, reverted_at
                FROM feedback_events WHERE feedback_id = ?1
                "#,
                [feedback_id],
                feedback_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_feedback_events(
        &self,
        profile_id: Option<&str>,
    ) -> DatabaseResult<Vec<FeedbackEventRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            r#"
            SELECT feedback_id, dataset_id, skill_id, query_id, other_skill_id,
                   relation_type, action, strength, before_state_json, after_state_json,
                   profile_id, source_evidence_json, confirmed_at, reverted_at
            FROM feedback_events
            WHERE ?1 IS NULL OR profile_id = ?1
            ORDER BY confirmed_at DESC, feedback_id
            "#,
        )?;
        let events = statement
            .query_map([profile_id], feedback_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(DatabaseError::from)?;
        Ok(events)
    }

    pub fn revert_feedback_bundle(
        &self,
        feedback_id: &str,
        reverted_at: i64,
        reversal_sample: Option<&ValidationSampleRecord>,
    ) -> DatabaseResult<FeedbackEventRecord> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let mut event = transaction
            .query_row(
                r#"
                SELECT feedback_id, dataset_id, skill_id, query_id, other_skill_id,
                       relation_type, action, strength, before_state_json, after_state_json,
                       profile_id, source_evidence_json, confirmed_at, reverted_at
                FROM feedback_events WHERE feedback_id = ?1
                "#,
                [feedback_id],
                feedback_from_row,
            )
            .optional()?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "feedback event",
                id: feedback_id.to_string(),
            })?;
        if event.reverted_at.is_some() {
            return Err(DatabaseError::InvalidValue {
                field: "feedback_events.reverted_at",
                value: "already reverted".to_string(),
            });
        }
        transaction.execute(
            "UPDATE feedback_events SET reverted_at = ?2 WHERE feedback_id = ?1",
            params![feedback_id, reverted_at],
        )?;
        transaction.execute(
            "UPDATE skill_relations SET state = 'human_reverted', validated_at = ?2 WHERE relation_id = ?1",
            params![format!("feedback:{feedback_id}"), reverted_at],
        )?;
        if let Some(sample) = reversal_sample {
            insert_validation_sample(&transaction, sample)?;
        }
        transaction.commit()?;
        event.reverted_at = Some(reverted_at);
        Ok(event)
    }

    pub fn validation_counts(&self) -> DatabaseResult<(u64, u64, u64)> {
        let connection = self.connection()?;
        Ok((
            table_count(&connection, "validation_samples")?,
            table_count(&connection, "validation_runs")?,
            table_count(&connection, "feedback_events")?,
        ))
    }

    pub fn delete_validation_sample(&self, sample_id: &str) -> DatabaseResult<bool> {
        Ok(self.connection()?.execute(
            "DELETE FROM validation_samples WHERE sample_id = ?1",
            [sample_id],
        )? > 0)
    }

    pub fn reset_local_validation_data(
        &self,
        profile_id: Option<&str>,
    ) -> DatabaseResult<LocalValidationMutation> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let relations = transaction.execute(
            r#"
            DELETE FROM skill_relations
            WHERE relation_id IN (
                SELECT 'feedback:' || feedback_id
                FROM feedback_events
                WHERE ?1 IS NULL OR profile_id = ?1
            )
            "#,
            [profile_id],
        )?;
        let feedback_events = transaction.execute(
            "DELETE FROM feedback_events WHERE ?1 IS NULL OR profile_id = ?1",
            [profile_id],
        )?;
        let runs = transaction.execute(
            "DELETE FROM validation_runs WHERE ?1 IS NULL OR profile_id = ?1",
            [profile_id],
        )?;
        let samples = transaction.execute(
            "DELETE FROM validation_samples WHERE ?1 IS NULL OR profile_id = ?1",
            [profile_id],
        )?;
        transaction.commit()?;
        Ok(LocalValidationMutation {
            samples: samples as u64,
            runs: runs as u64,
            feedback_events: feedback_events as u64,
            relations: relations as u64,
        })
    }

    pub fn recover_interrupted_work(&self, recovered_at: i64) -> DatabaseResult<RecoverySummary> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let jobs = transaction.execute(
            r#"
            UPDATE embedding_jobs
            SET status = 'failed',
                updated_at = ?1,
                error = 'interrupted_retryable'
            WHERE status IN ('pending', 'running', 'paused')
            "#,
            [recovered_at],
        )?;
        let runs = transaction.execute(
            r#"
            UPDATE validation_runs
            SET status = 'failed',
                completed_at = ?1,
                error = 'interrupted_retryable',
                metrics_json = '{"advisoryOnly":true,"blocksActivation":false,"confidence":"low","retryable":true,"warnings":["interrupted_before_completion"]}'
            WHERE status IN ('pending', 'running')
            "#,
            [recovered_at],
        )?;
        let profiles = transaction.execute(
            r#"
            UPDATE embedding_profiles
            SET status = 'failed', error = 'interrupted_retryable'
            WHERE status = 'building' AND is_active = 0
            "#,
            [],
        )?;
        transaction.commit()?;
        Ok(RecoverySummary {
            embedding_jobs: jobs as u64,
            validation_runs: runs as u64,
            building_profiles: profiles as u64,
        })
    }

    pub fn save_embedding(&self, vector: &StoredVector) -> DatabaseResult<()> {
        self.connection()?.execute(
            r#"
            INSERT INTO embeddings (
                embedding_id, profile_id, skill_id, vector_type, level,
                parent_embedding_id, resource_category, chunk_id, input_hash,
                vector_json, status, heading_path, source_file, generated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                unixepoch()
            )
            ON CONFLICT(embedding_id) DO UPDATE SET
                vector_json = excluded.vector_json,
                status = excluded.status,
                input_hash = excluded.input_hash,
                generated_at = excluded.generated_at
            "#,
            params![
                vector.embedding_id,
                vector.profile_id,
                vector.skill_id,
                vector.vector_type.as_str(),
                vector.level.as_str(),
                vector.parent_embedding_id,
                vector.resource_category,
                vector.chunk_id,
                vector.input_hash,
                serde_json::to_string(&vector.vector)?,
                vector.status.as_str(),
                vector.heading_path,
                vector.source_file,
            ],
        )?;
        Ok(())
    }

    pub fn load_vectors(&self, profile_id: &str) -> DatabaseResult<Vec<StoredVector>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            r#"
            SELECT embedding_id, profile_id, skill_id, vector_type, level,
                   parent_embedding_id, resource_category, chunk_id, input_hash,
                   vector_json, status, heading_path, source_file
            FROM embeddings
            WHERE profile_id = ?1 AND status IN ('ready', 'expired')
            ORDER BY embedding_id
            "#,
        )?;
        let rows = statement.query_map([profile_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
            ))
        })?;
        let mut vectors = Vec::new();
        for row in rows {
            let (
                embedding_id,
                profile_id,
                skill_id,
                vector_type,
                level,
                parent_embedding_id,
                resource_category,
                chunk_id,
                input_hash,
                vector,
                status,
                heading_path,
                source_file,
            ) = row?;
            vectors.push(StoredVector {
                embedding_id,
                profile_id,
                skill_id,
                vector_type: parse_value("embeddings.vector_type", &vector_type)?,
                level: parse_value("embeddings.level", &level)?,
                parent_embedding_id,
                resource_category,
                chunk_id,
                input_hash,
                vector: serde_json::from_str(&vector)?,
                status: parse_value("embeddings.status", &status)?,
                heading_path,
                source_file,
            });
        }
        Ok(vectors)
    }

    pub fn save_generated_inputs(
        &self,
        snapshot_id: &str,
        skill_id: &str,
        inputs: &[GeneratedInput],
        created_at: i64,
    ) -> DatabaseResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        for input in inputs {
            // Stable Skill identity migrations can leave an otherwise identical
            // input under its legacy input_id. Replace only that stale identity
            // before inserting the deterministic current input_id. Its chunks are
            // removed by the foreign-key cascade and rebuilt below.
            transaction.execute(
                r#"
                DELETE FROM embedding_inputs
                WHERE skill_id = ?1
                  AND vector_type = ?2
                  AND resource_category IS ?3
                  AND input_hash = ?4
                  AND input_id <> ?5
                "#,
                params![
                    skill_id,
                    input.parent.vector_type.as_str(),
                    input.parent.resource_category,
                    input.input_hash,
                    input.input_id,
                ],
            )?;
            transaction.execute(
                r#"
                INSERT INTO embedding_inputs (
                    input_id, skill_id, vector_type, resource_category, structured_json,
                    input_text, input_hash, schema_version, status, created_at, snapshot_id
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'ready', ?9, ?10)
                ON CONFLICT(input_id) DO UPDATE SET
                    structured_json = excluded.structured_json,
                    input_text = excluded.input_text,
                    input_hash = excluded.input_hash,
                    schema_version = excluded.schema_version,
                    status = 'ready',
                    created_at = excluded.created_at,
                    snapshot_id = excluded.snapshot_id
                "#,
                params![
                    input.input_id,
                    skill_id,
                    input.parent.vector_type.as_str(),
                    input.parent.resource_category,
                    serde_json::to_string(&input.parent)?,
                    input.text,
                    input.input_hash,
                    input.parent.schema_version,
                    created_at,
                    snapshot_id,
                ],
            )?;
            transaction.execute(
                "DELETE FROM embedding_chunks WHERE input_id = ?1",
                [&input.input_id],
            )?;
            for chunk in &input.chunks {
                transaction.execute(
                    r#"
                    INSERT INTO embedding_chunks (
                        chunk_id, input_id, relative_file_path, heading_path,
                        semantic_role, local_anchor, chunk_text, token_estimate,
                        input_hash, content_hash, created_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                    "#,
                    params![
                        chunk.chunk_id,
                        input.input_id,
                        chunk.relative_file_path,
                        chunk.heading_path,
                        chunk.semantic_role,
                        chunk.local_anchor,
                        chunk.text,
                        chunk.token_estimate as i64,
                        chunk.input_hash,
                        chunk.content_hash,
                        created_at,
                    ],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn has_ready_embedding(&self, profile_id: &str, input_hash: &str) -> DatabaseResult<bool> {
        self.connection()?
            .query_row(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM embeddings
                    WHERE profile_id = ?1 AND input_hash = ?2 AND status = 'ready'
                )
                "#,
                params![profile_id, input_hash],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn has_analysis_for_input(
        &self,
        skill_id: &str,
        input_hash: &str,
        require_llm: bool,
    ) -> DatabaseResult<bool> {
        let connection = self.connection()?;
        let rule_exists: bool = connection.query_row(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM analysis_rule_results
                WHERE skill_id = ?1 AND input_hash = ?2
            )
            "#,
            params![skill_id, input_hash],
            |row| row.get(0),
        )?;
        if !rule_exists || !require_llm {
            return Ok(rule_exists);
        }
        connection
            .query_row(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM analysis_llm_results
                    WHERE skill_id = ?1 AND input_hash = ?2
                )
                "#,
                params![skill_id, input_hash],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn expire_skill_vectors(
        &self,
        profile_id: &str,
        skill_id: &str,
        error: &str,
    ) -> DatabaseResult<()> {
        self.connection()?.execute(
            r#"
            UPDATE embeddings
            SET status = 'expired', error = ?3
            WHERE profile_id = ?1 AND skill_id = ?2 AND status = 'ready'
            "#,
            params![profile_id, skill_id, error],
        )?;
        Ok(())
    }

    pub fn remove_deleted_profile_skills(
        &self,
        profile_id: &str,
        current_skill_ids: &[String],
    ) -> DatabaseResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let existing = {
            let mut statement =
                transaction.prepare("SELECT skill_id FROM indexed_skills WHERE profile_id = ?1")?;
            let rows = statement
                .query_map([profile_id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        for skill_id in existing {
            if !current_skill_ids.contains(&skill_id) {
                transaction.execute(
                    "DELETE FROM embeddings WHERE profile_id = ?1 AND skill_id = ?2",
                    params![profile_id, skill_id],
                )?;
                transaction.execute(
                    "DELETE FROM indexed_skills WHERE profile_id = ?1 AND skill_id = ?2",
                    params![profile_id, skill_id],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn rebind_skill_identity(&self, old_id: &str, new_id: &str) -> DatabaseResult<()> {
        if old_id == new_id {
            return Ok(());
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        for statement in [
            "UPDATE embeddings SET skill_id = ?2 WHERE skill_id = ?1",
            "UPDATE embedding_inputs SET skill_id = ?2 WHERE skill_id = ?1",
            "UPDATE analysis_rule_results SET skill_id = ?2 WHERE skill_id = ?1",
            "UPDATE analysis_llm_results SET skill_id = ?2 WHERE skill_id = ?1",
            "UPDATE validation_samples SET skill_id = ?2 WHERE skill_id = ?1",
            "UPDATE feedback_events SET skill_id = ?2 WHERE skill_id = ?1",
            "UPDATE feedback_events SET other_skill_id = ?2 WHERE other_skill_id = ?1",
            "UPDATE skill_relations SET source_skill_id = ?2 WHERE source_skill_id = ?1",
            "UPDATE skill_relations SET target_skill_id = ?2 WHERE target_skill_id = ?1",
            "UPDATE indexed_skills SET skill_id = ?2 WHERE skill_id = ?1",
        ] {
            transaction.execute(statement, params![old_id, new_id])?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn save_analysis_results(
        &self,
        rule: &RuleAnalysisResult,
        llm: Option<&LlmAnalysisResult>,
        comparison: Option<&AnalysisComparison>,
        conflict: Option<&ConflictResolutionResult>,
    ) -> DatabaseResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            r#"
            INSERT OR REPLACE INTO analysis_rule_results (
                result_id, skill_id, parser_version, input_hash, result_json, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                rule.result_id,
                rule.skill_id,
                rule.parser_version,
                rule.input_hash,
                serde_json::to_string(rule)?,
                rule.created_at,
            ],
        )?;
        if let Some(llm) = llm {
            transaction.execute(
                r#"
                INSERT OR REPLACE INTO analysis_llm_results (
                    result_id, skill_id, provider, model, prompt_version,
                    prompt_text, schema_version, input_hash, raw_output_json,
                    result_json, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                "#,
                params![
                    llm.result_id,
                    llm.skill_id,
                    llm.provider,
                    llm.model,
                    llm.prompt_version,
                    llm.prompt,
                    llm.schema_version,
                    llm.input_hash,
                    serde_json::to_string(&llm.raw_output)?,
                    serde_json::to_string(llm)?,
                    llm.created_at,
                ],
            )?;
        }
        if let Some(comparison) = comparison {
            transaction.execute(
                r#"
                INSERT OR REPLACE INTO analysis_comparisons (
                    comparison_id, rule_result_id, llm_result_id, status,
                    result_json, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                params![
                    comparison.comparison_id,
                    comparison.rule_result_id,
                    comparison.llm_result_id,
                    format!("{:?}", comparison.status).to_ascii_lowercase(),
                    serde_json::to_string(comparison)?,
                    comparison.created_at,
                ],
            )?;
        }
        if let Some(conflict) = conflict {
            transaction.execute(
                r#"
                INSERT OR REPLACE INTO analysis_conflicts (
                    resolution_id, comparison_id, provider, model, prompt_version,
                    prompt_text, status, complete_input_json, raw_output_json, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                "#,
                params![
                    conflict.resolution_id,
                    conflict.comparison_id,
                    conflict.provider,
                    conflict.model,
                    conflict.prompt_version,
                    conflict.prompt,
                    format!("{:?}", conflict.status).to_ascii_lowercase(),
                    serde_json::to_string(&conflict.complete_input)?,
                    serde_json::to_string(&conflict.output)?,
                    conflict.created_at,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn create_job(&self, job: &EmbeddingJob) -> DatabaseResult<()> {
        self.connection()?.execute(
            r#"
            INSERT INTO embedding_jobs (
                job_id, profile_id, kind, status, total_items, completed_items,
                estimated_tokens, actual_tokens, estimated_cost_low,
                estimated_cost_high, created_at, updated_at, error, cancel_requested
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0)
            "#,
            params![
                job.job_id,
                job.profile_id,
                job.kind.as_str(),
                job.status.as_str(),
                job.total_items as i64,
                job.completed_items as i64,
                job.estimated_tokens as i64,
                job.actual_tokens as i64,
                job.estimated_cost_low,
                job.estimated_cost_high,
                job.created_at,
                job.updated_at,
                job.error,
            ],
        )?;
        Ok(())
    }

    pub fn save_job_context(
        &self,
        job_id: &str,
        context: &serde_json::Value,
    ) -> DatabaseResult<()> {
        self.connection()?.execute(
            "UPDATE embedding_jobs SET checkpoint_json = ?2 WHERE job_id = ?1",
            params![job_id, serde_json::to_string(context)?],
        )?;
        Ok(())
    }

    pub fn job_context(&self, job_id: &str) -> DatabaseResult<Option<serde_json::Value>> {
        let value = self
            .connection()?
            .query_row(
                "SELECT checkpoint_json FROM embedding_jobs WHERE job_id = ?1",
                [job_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        value
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    pub fn list_jobs(&self) -> DatabaseResult<Vec<EmbeddingJob>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            r#"
            SELECT job_id, profile_id, kind, status, total_items, completed_items,
                   estimated_tokens, actual_tokens, estimated_cost_low,
                   estimated_cost_high, created_at, updated_at, error
            FROM embedding_jobs ORDER BY created_at DESC, job_id
            "#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<f64>>(8)?,
                row.get::<_, Option<f64>>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, i64>(11)?,
                row.get::<_, Option<String>>(12)?,
            ))
        })?;
        let mut jobs = Vec::new();
        for row in rows {
            let row = row?;
            jobs.push(EmbeddingJob {
                job_id: row.0,
                profile_id: row.1,
                kind: parse_value("embedding_jobs.kind", &row.2)?,
                status: parse_value("embedding_jobs.status", &row.3)?,
                total_items: row.4.max(0) as u64,
                completed_items: row.5.max(0) as u64,
                estimated_tokens: row.6.max(0) as u64,
                actual_tokens: row.7.max(0) as u64,
                estimated_cost_low: row.8,
                estimated_cost_high: row.9,
                created_at: row.10,
                updated_at: row.11,
                error: row.12,
            });
        }
        Ok(jobs)
    }

    pub fn delete_job_history(&self, job_id: Option<&str>) -> DatabaseResult<u64> {
        let connection = self.connection()?;
        let deleted = if let Some(job_id) = job_id {
            connection.execute(
                "DELETE FROM embedding_jobs WHERE job_id = ?1 AND status IN ('completed', 'failed', 'cancelled')",
                [job_id],
            )?
        } else {
            connection.execute(
                "DELETE FROM embedding_jobs WHERE status IN ('completed', 'failed', 'cancelled')",
                [],
            )?
        };
        Ok(deleted as u64)
    }

    pub fn update_job(
        &self,
        job_id: &str,
        status: JobStatus,
        completed_items: u64,
        actual_tokens: u64,
        updated_at: i64,
        error: Option<&str>,
    ) -> DatabaseResult<()> {
        self.connection()?.execute(
            r#"
            UPDATE embedding_jobs
            SET status = ?2, completed_items = ?3, actual_tokens = ?4,
                updated_at = ?5, error = ?6
            WHERE job_id = ?1
            "#,
            params![
                job_id,
                status.as_str(),
                completed_items as i64,
                actual_tokens as i64,
                updated_at,
                error,
            ],
        )?;
        Ok(())
    }

    pub fn update_job_status(
        &self,
        job_id: &str,
        status: JobStatus,
        updated_at: i64,
        error: Option<&str>,
    ) -> DatabaseResult<()> {
        self.connection()?.execute(
            r#"
            UPDATE embedding_jobs
            SET status = ?2, updated_at = ?3, error = ?4
            WHERE job_id = ?1
            "#,
            params![job_id, status.as_str(), updated_at, error],
        )?;
        Ok(())
    }

    pub fn request_job_cancel(&self, job_id: &str) -> DatabaseResult<bool> {
        Ok(self.connection()?.execute(
            r#"
            UPDATE embedding_jobs SET cancel_requested = 1
            WHERE job_id = ?1 AND status IN ('pending', 'running', 'paused')
            "#,
            [job_id],
        )? > 0)
    }

    pub fn is_job_cancel_requested(&self, job_id: &str) -> DatabaseResult<bool> {
        self.connection()?
            .query_row(
                "SELECT cancel_requested FROM embedding_jobs WHERE job_id = ?1",
                [job_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn set_profile_status(
        &self,
        profile_id: &str,
        status: ProfileStatus,
        error: Option<&str>,
    ) -> DatabaseResult<()> {
        self.connection()?.execute(
            "UPDATE embedding_profiles SET status = ?2, error = ?3 WHERE profile_id = ?1",
            params![profile_id, status.as_str(), error],
        )?;
        Ok(())
    }

    pub fn compute_index_diff(
        &self,
        profile_id: &str,
        snapshot: &CanonicalSnapshot,
    ) -> DatabaseResult<IndexDiff> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT skill_id, content_hash FROM indexed_skills WHERE profile_id = ?1")?;
        let indexed = statement
            .query_map([profile_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<std::collections::HashMap<_, _>, _>>()?;
        let current = snapshot
            .skills
            .iter()
            .filter(|skill| !skill.path.starts_with("bundled://"))
            .map(|skill| (skill.skill_id.as_str(), skill.content_hash.as_str()))
            .collect::<std::collections::HashMap<_, _>>();
        let added = current
            .keys()
            .filter(|skill_id| !indexed.contains_key(**skill_id))
            .count() as u64;
        let changed = current
            .iter()
            .filter(|(skill_id, hash)| {
                indexed
                    .get(**skill_id)
                    .is_some_and(|indexed_hash| indexed_hash != **hash)
            })
            .count() as u64;
        let unchanged = current.len() as u64 - added - changed;
        let removed = indexed
            .keys()
            .filter(|skill_id| !current.contains_key(skill_id.as_str()))
            .count() as u64;
        Ok(IndexDiff {
            added,
            changed,
            removed,
            unchanged,
        })
    }

    pub fn unchanged_indexed_skill_ids(
        &self,
        profile_id: &str,
        snapshot: &CanonicalSnapshot,
    ) -> DatabaseResult<std::collections::HashSet<String>> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT skill_id, content_hash FROM indexed_skills WHERE profile_id = ?1")?;
        let indexed = statement
            .query_map([profile_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<std::collections::HashMap<_, _>, _>>()?;
        Ok(snapshot
            .skills
            .iter()
            .filter(|skill| !skill.path.starts_with("bundled://"))
            .filter(|skill| {
                indexed
                    .get(&skill.skill_id)
                    .is_some_and(|hash| hash == &skill.content_hash)
            })
            .map(|skill| skill.skill_id.clone())
            .collect())
    }

    pub fn replace_indexed_skills(
        &self,
        profile_id: &str,
        snapshot: &CanonicalSnapshot,
        synced_at: i64,
    ) -> DatabaseResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM indexed_skills WHERE profile_id = ?1",
            [profile_id],
        )?;
        for skill in snapshot
            .skills
            .iter()
            .filter(|skill| !skill.path.starts_with("bundled://"))
        {
            transaction.execute(
                r#"
                INSERT INTO indexed_skills (profile_id, skill_id, content_hash, indexed_at)
                VALUES (?1, ?2, ?3, ?4)
                "#,
                params![profile_id, skill.skill_id, skill.content_hash, synced_at],
            )?;
        }
        transaction.execute(
            r#"
            UPDATE index_settings
            SET last_synced_at = ?1, indexed_snapshot_id = ?2
            WHERE singleton_id = 1
            "#,
            params![synced_at, snapshot.snapshot_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_index_diff(&self, diff: &IndexDiff) -> DatabaseResult<()> {
        self.connection()?.execute(
            "UPDATE index_settings SET last_diff_json = ?1 WHERE singleton_id = 1",
            [serde_json::to_string(diff)?],
        )?;
        Ok(())
    }

    pub fn last_index_diff(&self) -> DatabaseResult<IndexDiff> {
        let value: String = self.connection()?.query_row(
            "SELECT last_diff_json FROM index_settings WHERE singleton_id = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(serde_json::from_str(&value)?)
    }

    pub fn set_index_auto_update(&self, enabled: bool) -> DatabaseResult<()> {
        self.connection()?.execute(
            "UPDATE index_settings SET auto_update = ?1 WHERE singleton_id = 1",
            [enabled],
        )?;
        Ok(())
    }

    pub fn index_sync_status(
        &self,
        active_profile_id: Option<&str>,
        pending_changes: u64,
    ) -> DatabaseResult<IndexSyncStatus> {
        let connection = self.connection()?;
        let (auto_update, last_synced_at): (bool, Option<i64>) = connection.query_row(
            "SELECT auto_update, last_synced_at FROM index_settings WHERE singleton_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let indexed_skills = if let Some(profile_id) = active_profile_id {
            connection.query_row(
                "SELECT COUNT(*) FROM indexed_skills WHERE profile_id = ?1",
                [profile_id],
                |row| row.get::<_, i64>(0),
            )?
        } else {
            0
        };
        Ok(IndexSyncStatus {
            auto_update,
            indexed_skills: indexed_skills.max(0) as u64,
            pending_changes,
            last_synced_at,
            active_profile_id: active_profile_id.map(str::to_string),
        })
    }

    pub fn replace_canonical_snapshot(&self, snapshot: &CanonicalSnapshot) -> DatabaseResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            r#"
            INSERT INTO canonical_snapshots (
                snapshot_id, content_hash, is_current, scan_started_at,
                scan_completed_at, searched_paths_json, warnings_json, skill_count,
                file_count, created_at
            ) VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6, ?7, ?8, ?4)
            "#,
            params![
                snapshot.snapshot_id,
                snapshot.content_hash,
                snapshot.scan_started_at,
                snapshot.scan_completed_at,
                serde_json::to_string(&snapshot.searched_paths)?,
                serde_json::to_string(&snapshot.warnings)?,
                snapshot.skills.len() as i64,
                snapshot
                    .skills
                    .iter()
                    .map(|skill| skill.files.len())
                    .sum::<usize>() as i64,
            ],
        )?;

        for skill in &snapshot.skills {
            transaction.execute(
                r#"
                INSERT INTO canonical_skills (
                    snapshot_id, skill_id, canonical_path, name, description,
                    content_hash, source_path, scope, is_built_in, in_library,
                    library_path, deleted_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL
                )
                "#,
                params![
                    snapshot.snapshot_id,
                    skill.skill_id,
                    skill.path,
                    skill.name,
                    skill.description,
                    skill.content_hash,
                    skill.source_path,
                    skill.scope,
                    skill.is_built_in,
                    skill.in_library,
                    skill.library_path,
                ],
            )?;
            for agent in &skill.enabled_agents {
                transaction.execute(
                    r#"
                    INSERT INTO canonical_agents (agent_id, name)
                    VALUES (?1, ?1)
                    ON CONFLICT(agent_id) DO UPDATE SET name = excluded.name
                    "#,
                    [agent],
                )?;
                transaction.execute(
                    r#"
                    INSERT INTO canonical_skill_agents (
                        snapshot_id, skill_id, agent_id, scope
                    ) VALUES (?1, ?2, ?3, ?4)
                    "#,
                    params![snapshot.snapshot_id, skill.skill_id, agent, skill.scope],
                )?;
            }
            for file in &skill.files {
                transaction.execute(
                    r#"
                    INSERT INTO canonical_files (
                        snapshot_id, file_id, skill_id, relative_path, media_type,
                        extension, content_hash, size_bytes, is_embeddable, modified_at,
                        semantic_role
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                    "#,
                    params![
                        snapshot.snapshot_id,
                        file.file_id,
                        skill.skill_id,
                        file.relative_path,
                        file.media_type,
                        file.extension,
                        file.content_hash,
                        file.size_bytes,
                        file.is_embeddable,
                        file.modified_at,
                        file.semantic_role,
                    ],
                )?;
            }
        }

        transaction.execute(
            "UPDATE canonical_snapshots SET is_current = 0 WHERE is_current = 1",
            [],
        )?;
        transaction.execute(
            "UPDATE canonical_snapshots SET is_current = 1 WHERE snapshot_id = ?1",
            [&snapshot.snapshot_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn current_canonical_snapshot(&self) -> DatabaseResult<Option<CanonicalSnapshot>> {
        let connection = self.connection()?;
        let metadata = connection
            .query_row(
                r#"
                SELECT snapshot_id, content_hash, scan_started_at, scan_completed_at,
                       searched_paths_json, warnings_json
                FROM canonical_snapshots
                WHERE is_current = 1
                "#,
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            snapshot_id,
            content_hash,
            scan_started_at,
            scan_completed_at,
            searched_paths_json,
            warnings_json,
        )) = metadata
        else {
            return Ok(None);
        };

        let mut skill_statement = connection.prepare(
            r#"
            SELECT skill_id, name, description, canonical_path, source_path, scope,
                   is_built_in, in_library, library_path, content_hash
            FROM canonical_skills
            WHERE snapshot_id = ?1 AND deleted_at IS NULL
            ORDER BY lower(name), content_hash, skill_id
            "#,
        )?;
        let skill_rows = skill_statement.query_map([&snapshot_id], |row| {
            Ok(CanonicalSkill {
                skill_id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                path: row.get(3)?,
                source_path: row.get(4)?,
                scope: row.get(5)?,
                is_built_in: row.get(6)?,
                enabled_agents: Vec::new(),
                in_library: row.get(7)?,
                library_path: row.get(8)?,
                content_hash: row.get(9)?,
                files: Vec::new(),
            })
        })?;
        let mut skills = skill_rows.collect::<Result<Vec<_>, _>>()?;

        let mut membership_statement = connection.prepare(
            r#"
            SELECT agent_id
            FROM canonical_skill_agents
            WHERE snapshot_id = ?1 AND skill_id = ?2
            ORDER BY agent_id
            "#,
        )?;
        let mut file_statement = connection.prepare(
            r#"
            SELECT file_id, relative_path, media_type, extension, content_hash,
                   size_bytes, is_embeddable, modified_at, semantic_role
            FROM canonical_files
            WHERE snapshot_id = ?1 AND skill_id = ?2
            ORDER BY relative_path
            "#,
        )?;
        for skill in &mut skills {
            skill.enabled_agents = membership_statement
                .query_map(params![snapshot_id, skill.skill_id], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            skill.files = file_statement
                .query_map(params![snapshot_id, skill.skill_id], |row| {
                    Ok(CanonicalFile {
                        file_id: row.get(0)?,
                        relative_path: row.get(1)?,
                        media_type: row.get(2)?,
                        extension: row.get(3)?,
                        content_hash: row.get(4)?,
                        size_bytes: row.get(5)?,
                        is_embeddable: row.get(6)?,
                        modified_at: row.get(7)?,
                        semantic_role: row.get(8)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
        }
        Ok(Some(CanonicalSnapshot {
            snapshot_id,
            content_hash,
            scan_started_at,
            scan_completed_at,
            searched_paths: serde_json::from_str(&searched_paths_json)?,
            warnings: serde_json::from_str(&warnings_json)?,
            skills,
        }))
    }

    #[cfg(test)]
    fn table_names(&self) -> DatabaseResult<Vec<String>> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?;
        let tables = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(DatabaseError::from)?;
        Ok(tables)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdaptiveHistoryRecord {
    pub history_id: String,
    pub profile_id: String,
    pub parameter_version: String,
    pub proposal: serde_json::Value,
    pub evidence: serde_json::Value,
    pub decision: String,
    pub created_at: i64,
    pub applied_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CredentialPurposeBinding {
    pub credential_id: String,
    pub purpose: String,
    pub provider: String,
    pub is_active: bool,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationSampleRecord {
    pub sample_id: String,
    pub dataset_id: String,
    pub dataset_schema_version: String,
    pub dataset_content_version: String,
    pub split: String,
    pub skill_id: Option<String>,
    pub chunk_id: Option<String>,
    pub query_text: String,
    pub query_language: String,
    pub task_type: String,
    pub label: String,
    pub state: ValidationState,
    pub evidence: serde_json::Value,
    pub confidence: Option<f64>,
    pub profile_id: Option<String>,
    pub input_hash: String,
    pub created_at: i64,
    pub reviewed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationRunRecord {
    pub run_id: String,
    pub dataset_id: String,
    pub dataset_content_version: String,
    pub profile_id: String,
    pub status: String,
    pub metrics: serde_json::Value,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackEventRecord {
    pub feedback_id: String,
    pub dataset_id: String,
    pub skill_id: Option<String>,
    pub query_id: Option<String>,
    pub other_skill_id: Option<String>,
    pub relation_type: Option<String>,
    pub action: String,
    pub strength: Option<f64>,
    pub before_state: serde_json::Value,
    pub after_state: serde_json::Value,
    pub profile_id: Option<String>,
    pub source_evidence: serde_json::Value,
    pub confirmed_at: i64,
    pub reverted_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalValidationMutation {
    pub samples: u64,
    pub runs: u64,
    pub feedback_events: u64,
    pub relations: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecoverySummary {
    pub embedding_jobs: u64,
    pub validation_runs: u64,
    pub building_profiles: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalSnapshot {
    pub snapshot_id: String,
    pub content_hash: String,
    pub scan_started_at: i64,
    pub scan_completed_at: i64,
    pub searched_paths: Vec<String>,
    pub warnings: Vec<String>,
    pub skills: Vec<CanonicalSkill>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalSkill {
    pub skill_id: String,
    pub name: String,
    pub description: Option<String>,
    pub path: String,
    pub source_path: String,
    pub scope: String,
    pub is_built_in: bool,
    pub enabled_agents: Vec<String>,
    pub in_library: bool,
    pub library_path: Option<String>,
    pub content_hash: String,
    pub files: Vec<CanonicalFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalFile {
    pub file_id: String,
    pub relative_path: String,
    pub media_type: Option<String>,
    pub extension: Option<String>,
    pub content_hash: String,
    pub size_bytes: i64,
    pub is_embeddable: bool,
    pub modified_at: Option<i64>,
    pub semantic_role: Option<String>,
}

fn migrate(connection: &mut Connection) -> DatabaseResult<()> {
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
        );
        "#,
    )?;
    let version: i64 = connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )?;
    if version < 1 {
        migration_v1(connection.transaction()?)?;
    }
    if version < 2 {
        migration_v2(connection.transaction()?)?;
    }
    if version < 3 {
        migration_v3(connection.transaction()?)?;
    }
    if version < 4 {
        migration_v4(connection.transaction()?)?;
    }
    let final_version: i64 = connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )?;
    if final_version != CURRENT_SCHEMA_VERSION {
        return Err(DatabaseError::InvalidValue {
            field: "schema_version",
            value: final_version.to_string(),
        });
    }
    Ok(())
}

fn migration_v1(transaction: Transaction<'_>) -> DatabaseResult<()> {
    transaction.execute_batch(
        r#"
        CREATE TABLE canonical_snapshots (
            snapshot_id TEXT PRIMARY KEY,
            content_hash TEXT NOT NULL,
            is_current INTEGER NOT NULL DEFAULT 0 CHECK (is_current IN (0, 1)),
            created_at INTEGER NOT NULL
        );

        CREATE TABLE canonical_skills (
            skill_id TEXT PRIMARY KEY,
            snapshot_id TEXT NOT NULL REFERENCES canonical_snapshots(snapshot_id) ON DELETE CASCADE,
            canonical_path TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT,
            content_hash TEXT NOT NULL,
            deleted_at INTEGER,
            UNIQUE(snapshot_id, canonical_path)
        );

        CREATE TABLE canonical_agents (
            agent_id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            official_documentation TEXT
        );

        CREATE TABLE canonical_skill_agents (
            skill_id TEXT NOT NULL REFERENCES canonical_skills(skill_id) ON DELETE CASCADE,
            agent_id TEXT NOT NULL REFERENCES canonical_agents(agent_id) ON DELETE CASCADE,
            scope TEXT NOT NULL,
            PRIMARY KEY(skill_id, agent_id)
        );

        CREATE TABLE canonical_files (
            file_id TEXT PRIMARY KEY,
            skill_id TEXT NOT NULL REFERENCES canonical_skills(skill_id) ON DELETE CASCADE,
            relative_path TEXT NOT NULL,
            media_type TEXT,
            content_hash TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            embeddable_text TEXT,
            semantic_role TEXT,
            UNIQUE(skill_id, relative_path)
        );

        CREATE TABLE credential_purpose_bindings (
            credential_id TEXT NOT NULL,
            purpose TEXT NOT NULL CHECK (purpose IN ('agent', 'embedding')),
            provider TEXT NOT NULL,
            is_active INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0, 1)),
            updated_at INTEGER NOT NULL,
            PRIMARY KEY(credential_id, purpose)
        );

        CREATE TABLE embedding_profiles (
            profile_id TEXT PRIMARY KEY,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            model_version TEXT NOT NULL,
            dimensions INTEGER NOT NULL CHECK (dimensions > 0),
            input_schema_version TEXT NOT NULL,
            chunk_policy_version TEXT NOT NULL,
            tokenizer TEXT,
            credential_id TEXT,
            status TEXT NOT NULL CHECK (
                status IN ('draft', 'building', 'ready', 'active', 'failed', 'archived')
            ),
            is_active INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0, 1)),
            created_at INTEGER NOT NULL,
            activated_at INTEGER,
            error TEXT
        );

        CREATE TABLE embedding_inputs (
            input_id TEXT PRIMARY KEY,
            skill_id TEXT NOT NULL,
            vector_type TEXT NOT NULL,
            resource_category TEXT,
            structured_json TEXT NOT NULL,
            input_text TEXT NOT NULL,
            input_hash TEXT NOT NULL,
            schema_version TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'ready',
            created_at INTEGER NOT NULL,
            UNIQUE(skill_id, vector_type, resource_category, input_hash)
        );

        CREATE TABLE embedding_chunks (
            chunk_id TEXT PRIMARY KEY,
            input_id TEXT NOT NULL REFERENCES embedding_inputs(input_id) ON DELETE CASCADE,
            relative_file_path TEXT NOT NULL,
            heading_path TEXT NOT NULL,
            semantic_role TEXT NOT NULL,
            local_anchor TEXT NOT NULL,
            chunk_text TEXT NOT NULL,
            token_estimate INTEGER NOT NULL,
            input_hash TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE embeddings (
            embedding_id TEXT PRIMARY KEY,
            profile_id TEXT NOT NULL REFERENCES embedding_profiles(profile_id) ON DELETE CASCADE,
            input_id TEXT REFERENCES embedding_inputs(input_id) ON DELETE CASCADE,
            skill_id TEXT NOT NULL,
            vector_type TEXT NOT NULL,
            level TEXT NOT NULL CHECK (level IN ('parent', 'chunk')),
            parent_embedding_id TEXT REFERENCES embeddings(embedding_id) ON DELETE CASCADE,
            resource_category TEXT,
            chunk_id TEXT REFERENCES embedding_chunks(chunk_id) ON DELETE CASCADE,
            input_hash TEXT NOT NULL,
            content_hash TEXT,
            source_file TEXT,
            heading_path TEXT,
            semantic_role TEXT,
            vector_json TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('pending', 'ready', 'expired', 'failed')),
            generated_at INTEGER NOT NULL,
            expires_at INTEGER,
            error TEXT
        );

        CREATE TABLE embedding_jobs (
            job_id TEXT PRIMARY KEY,
            profile_id TEXT NOT NULL REFERENCES embedding_profiles(profile_id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            status TEXT NOT NULL,
            total_items INTEGER NOT NULL DEFAULT 0,
            completed_items INTEGER NOT NULL DEFAULT 0,
            estimated_tokens INTEGER NOT NULL DEFAULT 0,
            actual_tokens INTEGER NOT NULL DEFAULT 0,
            estimated_cost_low REAL,
            estimated_cost_high REAL,
            checkpoint_json TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            error TEXT
        );

        CREATE TABLE skill_relations (
            relation_id TEXT PRIMARY KEY,
            source_skill_id TEXT NOT NULL,
            target_skill_id TEXT NOT NULL,
            relationship_type TEXT NOT NULL,
            vector_type TEXT,
            source_profile_id TEXT REFERENCES embedding_profiles(profile_id) ON DELETE SET NULL,
            target_profile_id TEXT REFERENCES embedding_profiles(profile_id) ON DELETE SET NULL,
            score REAL,
            state TEXT NOT NULL,
            evidence_json TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            validated_at INTEGER,
            CHECK (source_skill_id <> target_skill_id)
        );

        CREATE TABLE adaptive_history (
            history_id TEXT PRIMARY KEY,
            profile_id TEXT NOT NULL REFERENCES embedding_profiles(profile_id) ON DELETE CASCADE,
            parameter_version TEXT NOT NULL,
            proposal_json TEXT NOT NULL,
            evidence_json TEXT NOT NULL,
            decision TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            applied_at INTEGER
        );

        CREATE TABLE validation_samples (
            sample_id TEXT PRIMARY KEY,
            dataset_id TEXT NOT NULL,
            dataset_schema_version TEXT NOT NULL,
            dataset_content_version TEXT NOT NULL,
            split TEXT NOT NULL CHECK (split IN ('tuning', 'validation', 'holdout')),
            skill_id TEXT,
            chunk_id TEXT,
            query_text TEXT NOT NULL,
            query_language TEXT NOT NULL,
            task_type TEXT NOT NULL,
            label TEXT NOT NULL,
            state TEXT NOT NULL,
            evidence_json TEXT NOT NULL,
            confidence REAL,
            profile_id TEXT REFERENCES embedding_profiles(profile_id) ON DELETE SET NULL,
            input_hash TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            reviewed_at INTEGER
        );

        CREATE TABLE validation_runs (
            run_id TEXT PRIMARY KEY,
            dataset_id TEXT NOT NULL,
            dataset_content_version TEXT NOT NULL,
            profile_id TEXT NOT NULL REFERENCES embedding_profiles(profile_id) ON DELETE CASCADE,
            status TEXT NOT NULL,
            metrics_json TEXT NOT NULL,
            started_at INTEGER NOT NULL,
            completed_at INTEGER,
            error TEXT
        );

        CREATE TABLE feedback_events (
            feedback_id TEXT PRIMARY KEY,
            dataset_id TEXT NOT NULL,
            skill_id TEXT,
            query_id TEXT,
            other_skill_id TEXT,
            relation_type TEXT,
            action TEXT NOT NULL,
            strength REAL,
            before_state_json TEXT NOT NULL,
            after_state_json TEXT NOT NULL,
            profile_id TEXT REFERENCES embedding_profiles(profile_id) ON DELETE SET NULL,
            source_evidence_json TEXT NOT NULL,
            confirmed_at INTEGER NOT NULL,
            reverted_at INTEGER
        );

        CREATE UNIQUE INDEX one_current_snapshot
            ON canonical_snapshots(is_current) WHERE is_current = 1;
        CREATE UNIQUE INDEX one_active_credential_per_purpose
            ON credential_purpose_bindings(purpose) WHERE is_active = 1;
        CREATE UNIQUE INDEX one_active_embedding_profile
            ON embedding_profiles(is_active) WHERE is_active = 1;
        CREATE INDEX canonical_skills_snapshot_idx
            ON canonical_skills(snapshot_id, deleted_at);
        CREATE INDEX canonical_files_skill_idx
            ON canonical_files(skill_id, semantic_role);
        CREATE INDEX embedding_inputs_skill_type_idx
            ON embedding_inputs(skill_id, vector_type, resource_category);
        CREATE INDEX embedding_chunks_input_idx
            ON embedding_chunks(input_id, semantic_role);
        CREATE INDEX embeddings_profile_pool_idx
            ON embeddings(profile_id, vector_type, level, status);
        CREATE INDEX embeddings_skill_idx
            ON embeddings(profile_id, skill_id, status);
        CREATE INDEX embedding_jobs_profile_status_idx
            ON embedding_jobs(profile_id, status, updated_at);
        CREATE INDEX skill_relations_source_idx
            ON skill_relations(source_skill_id, relationship_type, state);
        CREATE INDEX skill_relations_target_idx
            ON skill_relations(target_skill_id, relationship_type, state);
        CREATE INDEX adaptive_history_profile_idx
            ON adaptive_history(profile_id, created_at);
        CREATE INDEX validation_samples_dataset_idx
            ON validation_samples(dataset_id, dataset_content_version, split, task_type);
        CREATE INDEX validation_runs_profile_idx
            ON validation_runs(profile_id, started_at);
        CREATE INDEX feedback_events_dataset_idx
            ON feedback_events(dataset_id, confirmed_at, reverted_at);

        INSERT INTO schema_version(version, applied_at) VALUES (1, unixepoch());
        "#,
    )?;
    transaction.commit()?;
    Ok(())
}

fn migration_v2(transaction: Transaction<'_>) -> DatabaseResult<()> {
    transaction.execute_batch(
        r#"
        DROP TABLE canonical_skill_agents;
        DROP TABLE canonical_files;
        DROP TABLE canonical_skills;
        DROP TABLE canonical_snapshots;

        CREATE TABLE canonical_snapshots (
            snapshot_id TEXT PRIMARY KEY,
            content_hash TEXT NOT NULL,
            is_current INTEGER NOT NULL DEFAULT 0 CHECK (is_current IN (0, 1)),
            scan_started_at INTEGER NOT NULL,
            scan_completed_at INTEGER NOT NULL,
            searched_paths_json TEXT NOT NULL,
            warnings_json TEXT NOT NULL,
            skill_count INTEGER NOT NULL,
            file_count INTEGER NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE canonical_skills (
            snapshot_id TEXT NOT NULL REFERENCES canonical_snapshots(snapshot_id) ON DELETE CASCADE,
            skill_id TEXT NOT NULL,
            canonical_path TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT,
            content_hash TEXT NOT NULL,
            source_path TEXT NOT NULL,
            scope TEXT NOT NULL,
            is_built_in INTEGER NOT NULL CHECK (is_built_in IN (0, 1)),
            in_library INTEGER NOT NULL CHECK (in_library IN (0, 1)),
            library_path TEXT,
            deleted_at INTEGER,
            PRIMARY KEY(snapshot_id, skill_id),
            UNIQUE(snapshot_id, canonical_path)
        );

        CREATE TABLE canonical_skill_agents (
            snapshot_id TEXT NOT NULL,
            skill_id TEXT NOT NULL,
            agent_id TEXT NOT NULL REFERENCES canonical_agents(agent_id) ON DELETE CASCADE,
            scope TEXT NOT NULL,
            PRIMARY KEY(snapshot_id, skill_id, agent_id),
            FOREIGN KEY(snapshot_id, skill_id)
                REFERENCES canonical_skills(snapshot_id, skill_id) ON DELETE CASCADE
        );

        CREATE TABLE canonical_files (
            snapshot_id TEXT NOT NULL,
            file_id TEXT NOT NULL,
            skill_id TEXT NOT NULL,
            relative_path TEXT NOT NULL,
            media_type TEXT,
            extension TEXT,
            content_hash TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            is_embeddable INTEGER NOT NULL CHECK (is_embeddable IN (0, 1)),
            modified_at INTEGER,
            semantic_role TEXT,
            PRIMARY KEY(snapshot_id, file_id),
            UNIQUE(snapshot_id, skill_id, relative_path),
            FOREIGN KEY(snapshot_id, skill_id)
                REFERENCES canonical_skills(snapshot_id, skill_id) ON DELETE CASCADE
        );

        CREATE UNIQUE INDEX one_current_snapshot
            ON canonical_snapshots(is_current) WHERE is_current = 1;
        CREATE INDEX canonical_skills_snapshot_idx
            ON canonical_skills(snapshot_id, deleted_at);
        CREATE INDEX canonical_files_skill_idx
            ON canonical_files(snapshot_id, skill_id, semantic_role);
        CREATE INDEX canonical_skill_agents_agent_idx
            ON canonical_skill_agents(snapshot_id, agent_id, skill_id);

        INSERT INTO schema_version(version, applied_at) VALUES (2, unixepoch());
        "#,
    )?;
    transaction.commit()?;
    Ok(())
}

fn migration_v3(transaction: Transaction<'_>) -> DatabaseResult<()> {
    transaction.execute_batch(
        r#"
        ALTER TABLE embedding_inputs ADD COLUMN snapshot_id TEXT;
        ALTER TABLE embedding_jobs ADD COLUMN cancel_requested INTEGER NOT NULL DEFAULT 0
            CHECK (cancel_requested IN (0, 1));

        CREATE TABLE analysis_rule_results (
            result_id TEXT PRIMARY KEY,
            skill_id TEXT NOT NULL,
            parser_version TEXT NOT NULL,
            input_hash TEXT NOT NULL,
            result_json TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE analysis_llm_results (
            result_id TEXT PRIMARY KEY,
            skill_id TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            prompt_version TEXT NOT NULL,
            prompt_text TEXT NOT NULL,
            schema_version TEXT NOT NULL,
            input_hash TEXT NOT NULL,
            raw_output_json TEXT NOT NULL,
            result_json TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE analysis_comparisons (
            comparison_id TEXT PRIMARY KEY,
            rule_result_id TEXT NOT NULL REFERENCES analysis_rule_results(result_id) ON DELETE CASCADE,
            llm_result_id TEXT NOT NULL REFERENCES analysis_llm_results(result_id) ON DELETE CASCADE,
            status TEXT NOT NULL,
            result_json TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE analysis_conflicts (
            resolution_id TEXT PRIMARY KEY,
            comparison_id TEXT NOT NULL REFERENCES analysis_comparisons(comparison_id) ON DELETE CASCADE,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            prompt_version TEXT NOT NULL,
            prompt_text TEXT NOT NULL,
            status TEXT NOT NULL,
            complete_input_json TEXT NOT NULL,
            raw_output_json TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE indexed_skills (
            profile_id TEXT NOT NULL REFERENCES embedding_profiles(profile_id) ON DELETE CASCADE,
            skill_id TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            indexed_at INTEGER NOT NULL,
            PRIMARY KEY(profile_id, skill_id)
        );

        CREATE TABLE index_settings (
            singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
            auto_update INTEGER NOT NULL DEFAULT 0 CHECK (auto_update IN (0, 1)),
            last_synced_at INTEGER,
            indexed_snapshot_id TEXT,
            last_diff_json TEXT NOT NULL DEFAULT '{"added":0,"changed":0,"removed":0,"unchanged":0}'
        );

        INSERT INTO index_settings(singleton_id) VALUES (1);

        CREATE INDEX analysis_rule_skill_idx
            ON analysis_rule_results(skill_id, created_at);
        CREATE INDEX analysis_llm_skill_idx
            ON analysis_llm_results(skill_id, created_at);
        CREATE INDEX embedding_inputs_snapshot_idx
            ON embedding_inputs(snapshot_id, skill_id);
        CREATE INDEX indexed_skills_profile_idx
            ON indexed_skills(profile_id, content_hash);

        INSERT INTO schema_version(version, applied_at) VALUES (3, unixepoch());
        "#,
    )?;
    transaction.commit()?;
    Ok(())
}

fn migration_v4(transaction: Transaction<'_>) -> DatabaseResult<()> {
    transaction.execute_batch(
        r#"
        CREATE TABLE adaptive_policy_state (
            profile_id TEXT PRIMARY KEY REFERENCES embedding_profiles(profile_id) ON DELETE CASCADE,
            h INTEGER NOT NULL,
            m INTEGER NOT NULL,
            u INTEGER NOT NULL,
            l INTEGER NOT NULL,
            local_update_enabled INTEGER NOT NULL DEFAULT 0
                CHECK (local_update_enabled IN (0, 1)),
            policy_version TEXT NOT NULL,
            limit_source TEXT NOT NULL,
            limit_source_version TEXT NOT NULL,
            applied_history_id TEXT,
            effective_after INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            CHECK (h > u AND u > l AND l > 0),
            CHECK (u = h - m AND l = u - m)
        );

        CREATE INDEX adaptive_policy_updated_idx
            ON adaptive_policy_state(updated_at);

        INSERT INTO schema_version(version, applied_at) VALUES (4, unixepoch());
        "#,
    )?;
    transaction.commit()?;
    Ok(())
}

fn profile_from_raw(raw: RawProfile) -> DatabaseResult<EmbeddingProfile> {
    Ok(EmbeddingProfile {
        profile_id: raw.0,
        provider: raw.1,
        model: raw.2,
        model_version: raw.3,
        dimensions: raw.4,
        input_schema_version: raw.5,
        chunk_policy_version: raw.6,
        tokenizer: raw.7,
        credential_id: raw.8,
        status: parse_value("embedding_profiles.status", &raw.9)?,
        is_active: raw.10,
        created_at: raw.11,
        activated_at: raw.12,
        error: raw.13,
    })
}

fn insert_validation_sample(
    transaction: &Transaction<'_>,
    sample: &ValidationSampleRecord,
) -> DatabaseResult<()> {
    transaction.execute(
        r#"
        INSERT INTO validation_samples (
            sample_id, dataset_id, dataset_schema_version, dataset_content_version,
            split, skill_id, chunk_id, query_text, query_language, task_type,
            label, state, evidence_json, confidence, profile_id, input_hash,
            created_at, reviewed_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
            ?15, ?16, ?17, ?18
        )
        "#,
        params![
            sample.sample_id,
            sample.dataset_id,
            sample.dataset_schema_version,
            sample.dataset_content_version,
            sample.split,
            sample.skill_id,
            sample.chunk_id,
            sample.query_text,
            sample.query_language,
            sample.task_type,
            sample.label,
            sample.state.as_str(),
            serde_json::to_string(&sample.evidence)?,
            sample.confidence,
            sample.profile_id,
            sample.input_hash,
            sample.created_at,
            sample.reviewed_at,
        ],
    )?;
    Ok(())
}

fn feedback_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FeedbackEventRecord> {
    let before_state = row.get::<_, String>(8)?;
    let after_state = row.get::<_, String>(9)?;
    let source_evidence = row.get::<_, String>(11)?;
    let parse_json = |index, value: String| {
        serde_json::from_str(&value).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    };
    Ok(FeedbackEventRecord {
        feedback_id: row.get(0)?,
        dataset_id: row.get(1)?,
        skill_id: row.get(2)?,
        query_id: row.get(3)?,
        other_skill_id: row.get(4)?,
        relation_type: row.get(5)?,
        action: row.get(6)?,
        strength: row.get(7)?,
        before_state: parse_json(8, before_state)?,
        after_state: parse_json(9, after_state)?,
        profile_id: row.get(10)?,
        source_evidence: parse_json(11, source_evidence)?,
        confirmed_at: row.get(12)?,
        reverted_at: row.get(13)?,
    })
}

fn raw_profile_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawProfile> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
    ))
}

fn parse_value<T: FromStr<Err = String>>(field: &'static str, value: &str) -> DatabaseResult<T> {
    value.parse().map_err(|_| DatabaseError::InvalidValue {
        field,
        value: value.to_string(),
    })
}

fn parse_relationship_type(value: &str) -> DatabaseResult<RelationshipType> {
    match value {
        "similar_to" => Ok(RelationshipType::SimilarTo),
        "overlaps_with" => Ok(RelationshipType::OverlapsWith),
        "conflicts_with" => Ok(RelationshipType::ConflictsWith),
        "duplicate_candidate" => Ok(RelationshipType::DuplicateCandidate),
        "supersedes" => Ok(RelationshipType::Supersedes),
        "depends_on" => Ok(RelationshipType::DependsOn),
        "reads_reference" => Ok(RelationshipType::ReadsReference),
        "runs_script" => Ok(RelationshipType::RunsScript),
        "uses_asset" => Ok(RelationshipType::UsesAsset),
        "located_in" => Ok(RelationshipType::LocatedIn),
        _ => Err(DatabaseError::InvalidValue {
            field: "skill_relations.relationship_type",
            value: value.to_string(),
        }),
    }
}

fn parse_relationship_state(value: &str) -> DatabaseResult<RelationshipState> {
    match value {
        "carried_fact" => Ok(RelationshipState::CarriedFact),
        "human_confirmed" => Ok(RelationshipState::HumanConfirmed),
        "human_rejected" => Ok(RelationshipState::HumanRejected),
        "human_reverted" => Ok(RelationshipState::HumanReverted),
        "migration_hint" => Ok(RelationshipState::MigrationHint),
        "revalidated" => Ok(RelationshipState::Revalidated),
        "rejected" => Ok(RelationshipState::Rejected),
        "conflict" => Ok(RelationshipState::Conflict),
        _ => Err(DatabaseError::InvalidValue {
            field: "skill_relations.state",
            value: value.to_string(),
        }),
    }
}

fn table_count(connection: &Connection, table: &str) -> DatabaseResult<u64> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let count: i64 = connection
        .query_row(&sql, [], |row| row.get(0))
        .map_err(DatabaseError::from)?;
    Ok(count.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectorization::{
        GeneratedChunk, GeneratedInput, JobKind, JobStatus, ProfileStatus, StructuredParentInput,
        VectorLevel, VectorStatus, CHUNK_POLICY_VERSION, INPUT_SCHEMA_VERSION,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    fn profile(id: &str, status: ProfileStatus) -> EmbeddingProfile {
        EmbeddingProfile {
            profile_id: id.to_string(),
            provider: "test".to_string(),
            model: "deterministic".to_string(),
            model_version: "1".to_string(),
            dimensions: 3,
            input_schema_version: INPUT_SCHEMA_VERSION.to_string(),
            chunk_policy_version: CHUNK_POLICY_VERSION.to_string(),
            tokenizer: Some("heuristic-v1".to_string()),
            credential_id: None,
            status,
            is_active: false,
            created_at: 1,
            activated_at: None,
            error: None,
        }
    }

    fn canonical_snapshot(id: &str, name: &str) -> CanonicalSnapshot {
        CanonicalSnapshot {
            snapshot_id: id.to_string(),
            content_hash: format!("{id}-hash"),
            scan_started_at: 1,
            scan_completed_at: 2,
            searched_paths: vec!["skills".to_string()],
            warnings: vec!["test warning".to_string()],
            skills: vec![CanonicalSkill {
                skill_id: "skill-stable".to_string(),
                name: name.to_string(),
                description: Some("Description".to_string()),
                path: format!("skills/{name}"),
                source_path: "skills".to_string(),
                scope: "user".to_string(),
                is_built_in: false,
                enabled_agents: vec!["claude-code".to_string(), "cursor".to_string()],
                in_library: true,
                library_path: Some(format!("library/{name}")),
                content_hash: "skill-content".to_string(),
                files: vec![CanonicalFile {
                    file_id: "file-stable".to_string(),
                    relative_path: "SKILL.md".to_string(),
                    media_type: Some("text/markdown".to_string()),
                    extension: Some("md".to_string()),
                    content_hash: "file-content".to_string(),
                    size_bytes: 42,
                    is_embeddable: true,
                    modified_at: Some(1),
                    semantic_role: Some("skill_definition".to_string()),
                }],
            }],
        }
    }

    #[test]
    fn migrations_create_all_foundation_tables() {
        let database = Database::in_memory().expect("database should initialize");
        assert_eq!(database.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
        assert_eq!(database.validation_counts().unwrap(), (0, 0, 0));
        let tables = database.table_names().unwrap();
        for expected in [
            "canonical_snapshots",
            "canonical_skills",
            "canonical_agents",
            "canonical_files",
            "credential_purpose_bindings",
            "embedding_profiles",
            "embedding_inputs",
            "embedding_chunks",
            "embeddings",
            "embedding_jobs",
            "skill_relations",
            "adaptive_history",
            "adaptive_policy_state",
            "validation_samples",
            "validation_runs",
            "feedback_events",
            "analysis_rule_results",
            "analysis_llm_results",
            "analysis_comparisons",
            "analysis_conflicts",
            "indexed_skills",
            "index_settings",
        ] {
            assert!(tables.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn canonical_snapshot_replacement_is_atomic_and_round_trips_memberships() {
        let database = Database::in_memory().unwrap();
        let first = canonical_snapshot("snapshot-1", "first");
        database.replace_canonical_snapshot(&first).unwrap();
        assert_eq!(
            database.current_canonical_snapshot().unwrap(),
            Some(first.clone())
        );

        let second = canonical_snapshot("snapshot-2", "second");
        database.replace_canonical_snapshot(&second).unwrap();
        assert_eq!(
            database.current_canonical_snapshot().unwrap(),
            Some(second.clone())
        );
        assert_eq!(
            database
                .current_canonical_snapshot()
                .unwrap()
                .unwrap()
                .skills[0]
                .enabled_agents,
            vec!["claude-code", "cursor"]
        );

        let mut invalid = canonical_snapshot("snapshot-invalid", "invalid");
        let duplicate_file = invalid.skills[0].files[0].clone();
        invalid.skills[0].files.push(duplicate_file);
        assert!(database.replace_canonical_snapshot(&invalid).is_err());
        assert_eq!(
            database
                .current_canonical_snapshot()
                .unwrap()
                .unwrap()
                .snapshot_id,
            "snapshot-2"
        );
    }

    #[test]
    fn profile_activation_is_atomic_and_unique() {
        let database = Database::in_memory().unwrap();
        database
            .upsert_profile(&profile("profile-a", ProfileStatus::Ready))
            .unwrap();
        database
            .upsert_profile(&profile("profile-b", ProfileStatus::Ready))
            .unwrap();
        database.activate_profile_atomic("profile-a", 10).unwrap();
        assert_eq!(
            database.active_profile().unwrap().unwrap().profile_id,
            "profile-a"
        );
        database.activate_profile_atomic("profile-b", 20).unwrap();
        let active = database.active_profile().unwrap().unwrap();
        assert_eq!(active.profile_id, "profile-b");
        assert_eq!(active.status, ProfileStatus::Active);
        assert!(database
            .activate_profile_atomic("missing-profile", 30)
            .is_err());
        assert_eq!(
            database.active_profile().unwrap().unwrap().profile_id,
            "profile-b"
        );
    }

    #[test]
    fn profile_and_embedding_credential_switch_is_atomic() {
        let database = Database::in_memory().unwrap();
        let mut old = profile("profile-old", ProfileStatus::Ready);
        old.provider = "openai".to_string();
        old.credential_id = Some("credential-old".to_string());
        let mut target = profile("profile-target", ProfileStatus::Ready);
        target.provider = "qwen".to_string();
        target.credential_id = Some("credential-target".to_string());
        database.upsert_profile(&old).unwrap();
        database.upsert_profile(&target).unwrap();
        database.activate_profile_atomic("profile-old", 1).unwrap();
        database
            .activate_credential_binding("credential-old", "embedding", "openai", 1)
            .unwrap();

        assert!(database
            .activate_profile_and_embedding_credential_atomic(
                "profile-target",
                "wrong-credential",
                "qwen",
                2
            )
            .is_err());
        assert_eq!(
            database.active_profile().unwrap().unwrap().profile_id,
            "profile-old"
        );
        assert_eq!(
            database
                .active_credential_binding("embedding")
                .unwrap()
                .unwrap()
                .credential_id,
            "credential-old"
        );

        database
            .activate_profile_and_embedding_credential_atomic(
                "profile-target",
                "credential-target",
                "qwen",
                3,
            )
            .unwrap();
        assert_eq!(
            database.active_profile().unwrap().unwrap().profile_id,
            "profile-target"
        );
        assert_eq!(
            database
                .active_credential_binding("embedding")
                .unwrap()
                .unwrap()
                .credential_id,
            "credential-target"
        );
    }

    #[test]
    fn credential_bindings_and_profiles_remain_role_and_provider_isolated() {
        let database = Database::in_memory().unwrap();
        let mut openai = profile("profile-openai", ProfileStatus::Ready);
        openai.provider = "openai".to_string();
        openai.model = "text-embedding-3-small".to_string();
        openai.credential_id = Some("credential-openai".to_string());
        let mut qwen = profile("profile-qwen", ProfileStatus::Ready);
        qwen.provider = "qwen".to_string();
        qwen.model = "text-embedding-v4".to_string();
        qwen.credential_id = Some("credential-qwen".to_string());
        database.upsert_profile(&openai).unwrap();
        database.upsert_profile(&qwen).unwrap();

        database
            .activate_credential_binding("credential-agent", "agent", "anthropic", 1)
            .unwrap();
        database
            .activate_credential_binding("credential-openai", "embedding", "openai", 2)
            .unwrap();
        assert_eq!(
            database
                .active_credential_binding("agent")
                .unwrap()
                .unwrap()
                .credential_id,
            "credential-agent"
        );
        assert_eq!(
            database
                .active_credential_binding("embedding")
                .unwrap()
                .unwrap()
                .credential_id,
            "credential-openai"
        );

        database
            .activate_credential_binding("credential-qwen", "embedding", "qwen", 3)
            .unwrap();
        assert_eq!(
            database
                .active_credential_binding("agent")
                .unwrap()
                .unwrap()
                .credential_id,
            "credential-agent"
        );
        assert_eq!(database.list_profiles().unwrap().len(), 2);

        database
            .activate_profile_atomic("profile-openai", 4)
            .unwrap();
        database.detach_credential("credential-openai").unwrap();
        let detached = database.profile("profile-openai").unwrap().unwrap();
        assert!(detached.credential_id.is_none());
        assert_eq!(detached.error.as_deref(), Some("credential_deleted"));
        assert_eq!(
            database.profile("profile-qwen").unwrap().unwrap().provider,
            "qwen"
        );
    }

    #[test]
    fn index_diff_jobs_cancellation_and_failure_expiry_persist() {
        let database = Database::in_memory().unwrap();
        let profile = profile("profile-index", ProfileStatus::Ready);
        database.upsert_profile(&profile).unwrap();
        let snapshot = canonical_snapshot("snapshot-index", "indexed");
        let initial = database
            .compute_index_diff("profile-index", &snapshot)
            .unwrap();
        assert_eq!(
            initial,
            IndexDiff {
                added: 1,
                changed: 0,
                removed: 0,
                unchanged: 0
            }
        );
        database
            .replace_indexed_skills("profile-index", &snapshot, 10)
            .unwrap();
        assert_eq!(
            database
                .unchanged_indexed_skill_ids("profile-index", &snapshot)
                .unwrap(),
            std::collections::HashSet::from(["skill-stable".to_string()])
        );
        assert_eq!(
            database
                .compute_index_diff("profile-index", &snapshot)
                .unwrap()
                .unchanged,
            1
        );
        let mut changed = snapshot.clone();
        changed.skills[0].content_hash = "changed".to_string();
        assert!(database
            .unchanged_indexed_skill_ids("profile-index", &changed)
            .unwrap()
            .is_empty());
        assert_eq!(
            database
                .compute_index_diff("profile-index", &changed)
                .unwrap()
                .changed,
            1
        );

        let job = EmbeddingJob {
            job_id: "job-cancel".to_string(),
            profile_id: "profile-index".to_string(),
            kind: JobKind::Incremental,
            status: JobStatus::Pending,
            total_items: 1,
            completed_items: 0,
            estimated_tokens: 10,
            actual_tokens: 0,
            estimated_cost_low: None,
            estimated_cost_high: None,
            created_at: 1,
            updated_at: 1,
            error: None,
        };
        database.create_job(&job).unwrap();
        assert_eq!(database.delete_job_history(Some("job-cancel")).unwrap(), 0);
        assert!(database.request_job_cancel("job-cancel").unwrap());
        assert!(database.is_job_cancel_requested("job-cancel").unwrap());
        database
            .update_job("job-cancel", JobStatus::Cancelled, 0, 0, 2, None)
            .unwrap();
        assert_eq!(
            database.list_jobs().unwrap()[0].status,
            JobStatus::Cancelled
        );
        assert_eq!(database.delete_job_history(Some("job-cancel")).unwrap(), 1);
        assert!(database.list_jobs().unwrap().is_empty());

        let mut active_job = job.clone();
        active_job.job_id = "job-active".to_string();
        active_job.status = JobStatus::Running;
        let mut completed_job = job.clone();
        completed_job.job_id = "job-completed".to_string();
        completed_job.status = JobStatus::Completed;
        let mut failed_job = job.clone();
        failed_job.job_id = "job-failed".to_string();
        failed_job.status = JobStatus::Failed;
        database.create_job(&active_job).unwrap();
        database.create_job(&completed_job).unwrap();
        database.create_job(&failed_job).unwrap();
        assert_eq!(database.delete_job_history(None).unwrap(), 2);
        let remaining_jobs = database.list_jobs().unwrap();
        assert_eq!(remaining_jobs.len(), 1);
        assert_eq!(remaining_jobs[0].job_id, "job-active");

        let vector = StoredVector {
            embedding_id: "embedding-ready".to_string(),
            profile_id: "profile-index".to_string(),
            skill_id: "skill-stable".to_string(),
            vector_type: VectorType::Workflow,
            level: VectorLevel::Parent,
            parent_embedding_id: None,
            resource_category: None,
            chunk_id: None,
            input_hash: "input".to_string(),
            vector: vec![1.0, 0.0, 0.0],
            status: VectorStatus::Ready,
            heading_path: None,
            source_file: None,
        };
        database.save_embedding(&vector).unwrap();
        database
            .expire_skill_vectors("profile-index", "skill-stable", "provider failed")
            .unwrap();
        assert_eq!(
            database.load_vectors("profile-index").unwrap()[0].status,
            VectorStatus::Expired
        );
        database
            .remove_deleted_profile_skills("profile-index", &[])
            .unwrap();
        assert!(database.load_vectors("profile-index").unwrap().is_empty());
        assert_eq!(
            database
                .compute_index_diff("profile-index", &snapshot)
                .unwrap()
                .added,
            1
        );
    }

    #[test]
    fn analysis_cache_distinguishes_rule_only_from_llm_complete() {
        let database = Database::in_memory().unwrap();
        database
            .connection()
            .unwrap()
            .execute(
                r#"
                INSERT INTO analysis_rule_results (
                    result_id, skill_id, parser_version, input_hash, result_json, created_at
                ) VALUES ('rule-1', 'skill-1', '1', 'input-1', '{}', 1)
                "#,
                [],
            )
            .unwrap();

        assert!(database
            .has_analysis_for_input("skill-1", "input-1", false)
            .unwrap());
        assert!(!database
            .has_analysis_for_input("skill-1", "input-1", true)
            .unwrap());
        assert!(!database
            .has_analysis_for_input("skill-1", "input-changed", false)
            .unwrap());

        database
            .connection()
            .unwrap()
            .execute(
                r#"
                INSERT INTO analysis_llm_results (
                    result_id, skill_id, provider, model, prompt_version,
                    prompt_text, schema_version, input_hash, raw_output_json,
                    result_json, created_at
                ) VALUES (
                    'llm-1', 'skill-1', 'test', 'analysis', '1',
                    'prompt', '1', 'input-1', '{}', '{}', 2
                )
                "#,
                [],
            )
            .unwrap();
        assert!(database
            .has_analysis_for_input("skill-1", "input-1", true)
            .unwrap());
    }

    #[test]
    fn stable_identity_migration_rebinds_existing_derived_data() {
        let database = Database::in_memory().unwrap();
        let profile = profile("profile-rebind", ProfileStatus::Ready);
        database.upsert_profile(&profile).unwrap();
        let snapshot = canonical_snapshot("snapshot-rebind", "rebind");
        database
            .replace_indexed_skills(&profile.profile_id, &snapshot, 1)
            .unwrap();
        database
            .save_embedding(&StoredVector {
                embedding_id: "embedding-rebind".to_string(),
                profile_id: profile.profile_id.clone(),
                skill_id: "skill-stable".to_string(),
                vector_type: VectorType::OverallFunction,
                level: VectorLevel::Parent,
                parent_embedding_id: None,
                resource_category: None,
                chunk_id: None,
                input_hash: "input-rebind".to_string(),
                vector: vec![1.0, 0.0, 0.0],
                status: VectorStatus::Ready,
                heading_path: None,
                source_file: None,
            })
            .unwrap();

        database
            .rebind_skill_identity("skill-stable", "skill-stable-v2")
            .unwrap();
        assert_eq!(
            database.load_vectors(&profile.profile_id).unwrap()[0].skill_id,
            "skill-stable-v2"
        );
        let mut migrated_snapshot = snapshot;
        migrated_snapshot.skills[0].skill_id = "skill-stable-v2".to_string();
        assert_eq!(
            database
                .compute_index_diff(&profile.profile_id, &migrated_snapshot)
                .unwrap()
                .unchanged,
            1
        );
    }

    #[test]
    fn generated_inputs_replace_legacy_input_identity_after_skill_rebind() {
        let database = Database::in_memory().unwrap();
        let parent = StructuredParentInput {
            vector_type: VectorType::OverallFunction,
            resource_category: None,
            title: "Stable Skill".to_string(),
            fields: BTreeMap::new(),
            source_headings: vec!["Description".to_string()],
            schema_version: INPUT_SCHEMA_VERSION.to_string(),
        };
        let generated = |input_id: &str, chunk_id: &str| GeneratedInput {
            input_id: input_id.to_string(),
            parent: parent.clone(),
            text: "same deterministic input".to_string(),
            input_hash: "same-input-hash".to_string(),
            chunks: vec![GeneratedChunk {
                chunk_id: chunk_id.to_string(),
                vector_type: VectorType::OverallFunction,
                resource_category: None,
                relative_file_path: "SKILL.md".to_string(),
                heading_path: "Description".to_string(),
                semantic_role: "description".to_string(),
                local_anchor: "description".to_string(),
                text: "same deterministic input".to_string(),
                token_estimate: 3,
                input_hash: "same-chunk-hash".to_string(),
                content_hash: "same-content-hash".to_string(),
            }],
        };

        database
            .save_generated_inputs(
                "snapshot-old",
                "skill-old",
                &[generated("legacy-input-id", "legacy-chunk-id")],
                1,
            )
            .unwrap();
        database
            .rebind_skill_identity("skill-old", "skill-stable")
            .unwrap();

        database
            .save_generated_inputs(
                "snapshot-current",
                "skill-stable",
                &[generated("stable-input-id", "stable-chunk-id")],
                2,
            )
            .unwrap();

        let connection = database.connection().unwrap();
        let input_ids = connection
            .prepare("SELECT input_id FROM embedding_inputs ORDER BY input_id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let chunk_ids = connection
            .prepare("SELECT chunk_id FROM embedding_chunks ORDER BY chunk_id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(input_ids, vec!["stable-input-id"]);
        assert_eq!(chunk_ids, vec!["stable-chunk-id"]);
    }

    #[test]
    fn relationships_round_trip_with_evidence() {
        let database = Database::in_memory().unwrap();
        let relation = SkillRelationship {
            relation_id: "relation-1".to_string(),
            source_skill_id: "skill-a".to_string(),
            target_skill_id: "skill-b".to_string(),
            relationship_type: RelationshipType::OverlapsWith,
            vector_type: Some(VectorType::Workflow),
            source_profile_id: None,
            target_profile_id: None,
            score: Some(0.75),
            state: RelationshipState::CarriedFact,
            evidence: json!({"heading": "Workflow"}),
            created_at: 1,
            validated_at: None,
        };
        database.save_relationship(&relation).unwrap();
        assert_eq!(
            database.relationships_for_skill("skill-b").unwrap(),
            vec![relation]
        );
    }

    #[test]
    fn adaptive_and_validation_records_persist() {
        let database = Database::in_memory().unwrap();
        database
            .upsert_profile(&profile("profile-a", ProfileStatus::Ready))
            .unwrap();
        let history = AdaptiveHistoryRecord {
            history_id: "history-1".to_string(),
            profile_id: "profile-a".to_string(),
            parameter_version: "v2".to_string(),
            proposal: json!({"maxChunkTokens": 700}),
            evidence: json!({"sampleCount": 12}),
            decision: "proposed".to_string(),
            created_at: 2,
            applied_at: None,
        };
        database.append_adaptive_history(&history).unwrap();
        assert_eq!(
            database.adaptive_history("profile-a").unwrap(),
            vec![history]
        );

        database
            .save_validation_sample(&ValidationSampleRecord {
                sample_id: "sample-1".to_string(),
                dataset_id: "local-1".to_string(),
                dataset_schema_version: "1".to_string(),
                dataset_content_version: "1".to_string(),
                split: "validation".to_string(),
                skill_id: Some("skill-a".to_string()),
                chunk_id: None,
                query_text: "read a pdf".to_string(),
                query_language: "en".to_string(),
                task_type: "overall_function".to_string(),
                label: "3".to_string(),
                state: ValidationState::RuleDerived,
                evidence: json!({"field": "description"}),
                confidence: Some(1.0),
                profile_id: Some("profile-a".to_string()),
                input_hash: "abc".to_string(),
                created_at: 3,
                reviewed_at: None,
            })
            .unwrap();
        database
            .save_validation_run(&ValidationRunRecord {
                run_id: "run-1".to_string(),
                dataset_id: "local-1".to_string(),
                dataset_content_version: "1".to_string(),
                profile_id: "profile-a".to_string(),
                status: "completed".to_string(),
                metrics: json!({"recallAtK": 1.0}),
                started_at: 4,
                completed_at: Some(5),
                error: None,
            })
            .unwrap();
        database
            .save_feedback_event(&FeedbackEventRecord {
                feedback_id: "feedback-1".to_string(),
                dataset_id: "local-1".to_string(),
                skill_id: Some("skill-a".to_string()),
                query_id: Some("query-1".to_string()),
                other_skill_id: None,
                relation_type: Some("overall_function".to_string()),
                action: "rank_above".to_string(),
                strength: Some(1.0),
                before_state: json!({}),
                after_state: json!({"rank": 1}),
                profile_id: Some("profile-a".to_string()),
                source_evidence: json!({"explicit": true}),
                confirmed_at: 6,
                reverted_at: None,
            })
            .unwrap();
        assert_eq!(database.validation_counts().unwrap(), (1, 1, 1));
    }

    #[test]
    fn semantic_feedback_bundle_is_versioned_and_reversible() {
        let database = Database::in_memory().unwrap();
        database
            .upsert_profile(&profile("profile-feedback", ProfileStatus::Ready))
            .unwrap();
        let event = FeedbackEventRecord {
            feedback_id: "feedback-versioned".to_string(),
            dataset_id: "user-local:test".to_string(),
            skill_id: Some("skill-a".to_string()),
            query_id: Some("query-a".to_string()),
            other_skill_id: Some("skill-b".to_string()),
            relation_type: Some("similar_to".to_string()),
            action: "move_closer".to_string(),
            strength: Some(0.8),
            before_state: json!({}),
            after_state: json!({"semantic": true}),
            profile_id: Some("profile-feedback".to_string()),
            source_evidence: json!({"explicit": true}),
            confirmed_at: 10,
            reverted_at: None,
        };
        let relationship = SkillRelationship {
            relation_id: "feedback:feedback-versioned".to_string(),
            source_skill_id: "skill-a".to_string(),
            target_skill_id: "skill-b".to_string(),
            relationship_type: RelationshipType::SimilarTo,
            vector_type: None,
            source_profile_id: Some("profile-feedback".to_string()),
            target_profile_id: Some("profile-feedback".to_string()),
            score: Some(0.8),
            state: RelationshipState::HumanConfirmed,
            evidence: json!({"feedbackId": "feedback-versioned"}),
            created_at: 10,
            validated_at: Some(10),
        };
        database
            .record_feedback_bundle(&event, Some(&relationship), None)
            .unwrap();
        assert_eq!(
            database
                .feedback_event("feedback-versioned")
                .unwrap()
                .unwrap()
                .reverted_at,
            None
        );
        database
            .revert_feedback_bundle("feedback-versioned", 11, None)
            .unwrap();
        assert_eq!(
            database
                .feedback_event("feedback-versioned")
                .unwrap()
                .unwrap()
                .reverted_at,
            Some(11)
        );
        assert_eq!(
            database.relationships_for_skill("skill-a").unwrap()[0].state,
            RelationshipState::HumanReverted
        );
        assert!(database
            .revert_feedback_bundle("feedback-versioned", 12, None)
            .is_err());
    }

    #[test]
    fn startup_recovery_marks_remote_work_retryable_without_resuming() {
        let database = Database::in_memory().unwrap();
        database
            .upsert_profile(&profile("profile-interrupted", ProfileStatus::Building))
            .unwrap();
        database
            .create_job(&EmbeddingJob {
                job_id: "job-interrupted".to_string(),
                profile_id: "profile-interrupted".to_string(),
                kind: JobKind::FullRebuild,
                status: JobStatus::Running,
                total_items: 10,
                completed_items: 4,
                estimated_tokens: 100,
                actual_tokens: 40,
                estimated_cost_low: None,
                estimated_cost_high: None,
                created_at: 1,
                updated_at: 2,
                error: None,
            })
            .unwrap();
        database
            .save_validation_run(&ValidationRunRecord {
                run_id: "run-interrupted".to_string(),
                dataset_id: "user-local:test".to_string(),
                dataset_content_version: "v1".to_string(),
                profile_id: "profile-interrupted".to_string(),
                status: "pending".to_string(),
                metrics: json!({"advisoryOnly": true}),
                started_at: 2,
                completed_at: None,
                error: None,
            })
            .unwrap();
        let recovered = database.recover_interrupted_work(10).unwrap();
        assert_eq!(recovered.embedding_jobs, 1);
        assert_eq!(recovered.validation_runs, 1);
        assert_eq!(recovered.building_profiles, 1);
        let job = database.list_jobs().unwrap().remove(0);
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.error.as_deref(), Some("interrupted_retryable"));
        let run = database.list_validation_runs(None).unwrap().remove(0);
        assert_eq!(run.status, "failed");
        assert_eq!(run.metrics["retryable"], true);
        assert_eq!(
            database
                .profile("profile-interrupted")
                .unwrap()
                .unwrap()
                .status,
            ProfileStatus::Failed
        );
    }
}
