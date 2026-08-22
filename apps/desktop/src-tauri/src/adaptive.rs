use crate::vectorization::EmbeddingProfile;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const ADAPTIVE_POLICY_SCHEMA_VERSION: &str = "deadalus.adaptive-policy.v1";
pub const PROVIDER_LIMIT_SOURCE_VERSION: &str = "provider-limits-2026-08";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdaptivePolicyState {
    pub profile_id: String,
    pub h: u64,
    pub m: u64,
    pub u: u64,
    pub l: u64,
    pub local_update_enabled: bool,
    pub policy_version: String,
    pub limit_source: String,
    pub limit_source_version: String,
    pub applied_history_id: Option<String>,
    pub effective_after: i64,
    pub updated_at: i64,
}

impl AdaptivePolicyState {
    pub fn validate(&self) -> Result<(), String> {
        if !(self.h > self.u && self.u > self.l && self.l > 0) {
            return Err("adaptive bounds must satisfy H > U > L > 0".to_string());
        }
        if self.u != self.h.saturating_sub(self.m) || self.l != self.u.saturating_sub(self.m) {
            return Err("adaptive bounds must satisfy U=H-M and L=U-M".to_string());
        }
        let minimum_margin = minimum_safety_margin(self.h);
        if self.m < minimum_margin {
            return Err(format!(
                "adaptive margin is below model safety minimum {minimum_margin}"
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdaptiveSignals {
    pub token_lengths: Vec<u64>,
    pub parent_count: u64,
    pub chunk_count: u64,
    pub successful_calls: u64,
    pub failed_calls: u64,
    pub actual_tokens: u64,
    pub latest_quality_metrics: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdaptiveProposal {
    pub policy: AdaptivePolicyState,
    pub requires_review: bool,
    pub large_change: bool,
    pub effective_for: String,
    pub cost_diagnostic: Value,
    pub cost_veto: bool,
    pub confidence: String,
    pub warnings: Vec<String>,
}

pub fn initial_policy(profile: &EmbeddingProfile, now: i64) -> AdaptivePolicyState {
    let (h, limit_source) = model_max_input(&profile.provider, &profile.model);
    let m = conservative_margin(h);
    AdaptivePolicyState {
        profile_id: profile.profile_id.clone(),
        h,
        m,
        u: h - m,
        l: h - 2 * m,
        local_update_enabled: false,
        policy_version: ADAPTIVE_POLICY_SCHEMA_VERSION.to_string(),
        limit_source,
        limit_source_version: PROVIDER_LIMIT_SOURCE_VERSION.to_string(),
        applied_history_id: None,
        effective_after: now,
        updated_at: now,
    }
}

pub fn propose_policy(
    current: &AdaptivePolicyState,
    signals: &AdaptiveSignals,
    now: i64,
) -> AdaptiveProposal {
    let mut lengths = signals.token_lengths.clone();
    lengths.sort_unstable();
    let p95 = percentile(&lengths, 95);
    let call_total = signals.successful_calls + signals.failed_calls;
    let failure_rate = if call_total > 0 {
        signals.failed_calls as f64 / call_total as f64
    } else {
        0.0
    };
    let chunk_ratio = if signals.parent_count > 0 {
        signals.chunk_count as f64 / signals.parent_count as f64
    } else {
        0.0
    };
    let mut proposed_m = current.m;
    if let Some(p95) = p95 {
        if p95 > current.l.saturating_mul(9) / 10 {
            proposed_m = current.m.saturating_mul(4) / 5;
        } else if p95 < current.l / 2 && chunk_ratio < 1.0 {
            proposed_m = current.m.saturating_mul(6) / 5;
        }
    }
    proposed_m = proposed_m
        .max(minimum_safety_margin(current.h))
        .min((current.h.saturating_sub(1)) / 2);
    let mut warnings = Vec::new();
    if lengths.len() < 20 {
        warnings.push("insufficient_input_distribution".to_string());
    }
    if call_total < 5 {
        warnings.push("insufficient_call_outcomes".to_string());
    }
    if signals.latest_quality_metrics.is_none() {
        warnings.push("quality_signals_unavailable".to_string());
    }
    if failure_rate > 0.2 {
        warnings.push("call_failure_rate_veto".to_string());
        proposed_m = current.m;
    }
    let relative_change = current
        .m
        .checked_sub(proposed_m)
        .or_else(|| proposed_m.checked_sub(current.m))
        .map(|difference| difference as f64 / current.m.max(1) as f64)
        .unwrap_or(0.0);
    let large_change = relative_change > 0.1;
    if large_change {
        warnings.push("large_change_requires_explicit_review".to_string());
    }
    let policy = AdaptivePolicyState {
        m: proposed_m,
        u: current.h - proposed_m,
        l: current.h - 2 * proposed_m,
        policy_version: format!("{}:{}", ADAPTIVE_POLICY_SCHEMA_VERSION, now),
        applied_history_id: current.applied_history_id.clone(),
        effective_after: now,
        updated_at: now,
        ..current.clone()
    };
    AdaptiveProposal {
        policy,
        requires_review: true,
        large_change,
        effective_for: "jobs_created_after_explicit_apply".to_string(),
        cost_diagnostic: serde_json::json!({
            "actualTokens": signals.actual_tokens,
            "optimizationObjective": false,
            "usage": "diagnostic_or_veto_only",
        }),
        cost_veto: false,
        confidence: if lengths.len() < 20 || call_total < 5 {
            "low".to_string()
        } else if signals.latest_quality_metrics.is_none() {
            "medium".to_string()
        } else {
            "high".to_string()
        },
        warnings,
    }
}

pub fn model_max_input(provider: &str, model: &str) -> (u64, String) {
    match (provider, model) {
        ("openai", model) if model.starts_with("text-embedding-3-") => {
            (8_191, "openai-model-input-limit".to_string())
        }
        ("qwen", "text-embedding-v4") => (8_192, "qwen-model-input-limit".to_string()),
        ("openai", _) => (8_191, "openai-conservative-fallback".to_string()),
        ("qwen", _) => (8_192, "qwen-conservative-fallback".to_string()),
        _ => (4_096, "provider-conservative-fallback".to_string()),
    }
}

pub const fn minimum_safety_margin(h: u64) -> u64 {
    let fraction = h / 32;
    if fraction > 128 {
        fraction
    } else {
        128
    }
}

const fn conservative_margin(h: u64) -> u64 {
    let fraction = h / 8;
    if fraction > 256 {
        fraction
    } else {
        256
    }
}

fn percentile(sorted: &[u64], percentile: usize) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let index = ((sorted.len() - 1) * percentile) / 100;
    sorted.get(index).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectorization::{ProfileStatus, CHUNK_POLICY_VERSION, INPUT_SCHEMA_VERSION};

    fn profile() -> EmbeddingProfile {
        EmbeddingProfile {
            profile_id: "profile".to_string(),
            provider: "openai".to_string(),
            model: "text-embedding-3-small".to_string(),
            model_version: "1".to_string(),
            dimensions: 1536,
            input_schema_version: INPUT_SCHEMA_VERSION.to_string(),
            chunk_policy_version: CHUNK_POLICY_VERSION.to_string(),
            tokenizer: None,
            credential_id: None,
            status: ProfileStatus::Ready,
            is_active: false,
            created_at: 1,
            activated_at: None,
            error: None,
        }
    }

    #[test]
    fn initial_formula_and_safety_margin_are_valid() {
        let policy = initial_policy(&profile(), 1);
        assert_eq!(policy.h, 8_191);
        assert_eq!(policy.u, policy.h - policy.m);
        assert_eq!(policy.l, policy.u - policy.m);
        policy.validate().unwrap();
    }

    #[test]
    fn sparse_large_proposal_stays_pending_for_later_jobs() {
        let current = initial_policy(&profile(), 1);
        let proposal = propose_policy(
            &current,
            &AdaptiveSignals {
                token_lengths: vec![10; 3],
                parent_count: 10,
                chunk_count: 0,
                successful_calls: 1,
                failed_calls: 0,
                actual_tokens: 30,
                latest_quality_metrics: None,
            },
            2,
        );
        assert!(proposal.requires_review);
        assert!(proposal.large_change);
        assert_eq!(proposal.effective_for, "jobs_created_after_explicit_apply");
        assert_eq!(proposal.confidence, "low");
        assert_eq!(proposal.cost_diagnostic["optimizationObjective"], false);
        assert_eq!(current.m, initial_policy(&profile(), 1).m);
    }
}
