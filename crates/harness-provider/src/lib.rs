//! Provider-neutral model boundary over HTTP.
//!
//! The runtime owns permissions, budgets, persistence, and verification.
//! Adapters in this crate only propose strictly validated `harness_core`
//! decisions. Endpoint configuration lives in memory; credentials are read
//! from the environment per call and never enter prompts, decisions, events,
//! or checkpoints.

pub mod fetch;
pub mod openai;
pub mod radar;

pub use fetch::{FetchLimits, FetchTool};
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
    /// Observation bytes forwarded per entry; the rest is cut at a UTF-8
    /// character boundary (see [`truncate_to_char_boundary`]).
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

/// Truncate `data` to at most `limit` bytes without splitting a UTF-8
/// character, returning whether truncation occurred.
///
/// `String::truncate` panics when `limit` lands inside a multi-byte
/// character (e.g. Vietnamese diacritics or emoji). This floors `limit` to
/// the nearest character boundary first, so callers can safely bound
/// model-prompt history and evidence strings.
pub fn truncate_to_char_boundary(data: &mut String, limit: usize) -> bool {
    if data.len() <= limit {
        return false;
    }
    let mut end = limit.min(data.len());
    while !data.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    data.truncate(end);
    true
}

#[cfg(test)]
mod truncate_tests {
    use super::truncate_to_char_boundary;

    #[test]
    fn ascii_truncates_at_limit() {
        let mut data = String::from("hello world");
        assert!(truncate_to_char_boundary(&mut data, 5));
        assert_eq!(data, "hello");
    }

    #[test]
    fn vietnamese_never_panics_or_splits() {
        // "Tiếng Việt" mixes precomposed and combining sequences.
        let source = "Tiếng Việt có dấu: ễ ộ ữ — test".repeat(4);
        for limit in [1, 2, 3, 5, 7, 10, 17, 32] {
            let mut data = source.clone();
            let cut = truncate_to_char_boundary(&mut data, limit);
            assert!(cut, "limit {limit} should truncate");
            assert!(data.len() <= limit, "limit {limit} got {}", data.len());
            assert!(str::from_utf8(data.as_bytes()).is_ok());
        }
    }

    #[test]
    fn emoji_never_panics_or_splits() {
        let source = "😀🎉🧪🚀✨".repeat(8);
        for limit in [1, 2, 3, 4, 5, 7, 9, 15] {
            let mut data = source.clone();
            truncate_to_char_boundary(&mut data, limit);
            assert!(data.len() <= limit);
            assert!(str::from_utf8(data.as_bytes()).is_ok());
        }
    }

    #[test]
    fn mixed_scripts_stay_valid() {
        let source = "hello Tiếng Việt 😀 mixed 123 — test ".repeat(6);
        let mut data = source.clone();
        assert!(truncate_to_char_boundary(&mut data, 25));
        assert!(data.len() <= 25);
        assert!(str::from_utf8(data.as_bytes()).is_ok());
        // Short content under the limit is untouched.
        let mut tiny = String::from("ok 😀");
        assert!(!truncate_to_char_boundary(&mut tiny, 64));
        assert_eq!(tiny, "ok 😀");
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
