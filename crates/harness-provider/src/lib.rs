//! Provider-neutral model boundary over HTTP.
//!
//! The runtime owns permissions, budgets, persistence, and verification.
//! Adapters in this crate only propose strictly validated `harness_core`
//! decisions. Endpoint configuration lives in memory; credentials are read
//! from the environment per call and never enter prompts, decisions, events,
//! or checkpoints.

pub mod openai;

pub use openai::{OpenAiCompat, Pricing, ProviderConfig};

use std::time::Duration;

/// Bounds for one provider call sequence. Every bound fails closed: the
/// adapter returns [`harness_core::StepDecision::Fail`] instead of guessing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderLimits {
    /// Wall-clock timeout for a single HTTP attempt.
    pub timeout_per_attempt: Duration,
    /// Maximum response body accepted before the call is rejected.
    pub max_response_bytes: usize,
    /// Retries after the first attempt, only for transport errors and
    /// HTTP 429/500/502/503/504, with bounded backoff.
    pub max_retries: u32,
    /// History entries (action, observation) sent per call, newest last.
    pub max_history_entries: usize,
    /// Observation characters forwarded per entry; the rest is cut.
    pub max_observation_chars: usize,
}

impl Default for ProviderLimits {
    fn default() -> Self {
        Self {
            timeout_per_attempt: Duration::from_secs(60),
            max_response_bytes: 256 * 1024,
            max_retries: 2,
            max_history_entries: 20,
            max_observation_chars: 2000,
        }
    }
}

/// Client-side usage accounting. The runtime does not yet persist provider
/// usage into events; callers read this after a run for budgets and reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub http_calls: u64,
    pub retries: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}
