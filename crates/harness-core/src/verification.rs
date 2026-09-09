//! Success criteria belong to the harness protocol, independently of any model.
use crate::types::{Action, Observation, Verification};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuccessCriterion {
    FileContents {
        path: String,
        expected: String,
    },
    FileDigest {
        path: String,
        sha256: String,
    },
    FileRange {
        path: String,
        offset: u64,
        expected: String,
    },
}

impl SuccessCriterion {
    /// Parse the CLI's deterministic objective grammar into an explicit claim.
    pub fn from_objective(text: &str) -> Option<Self> {
        let trimmed = text.trim();
        let lower = trimmed.to_ascii_lowercase();
        let (path, expected) = if let Some(rest) = lower.strip_prefix("create file ") {
            let marker = " with content ";
            let index = rest.find(marker)?;
            let end = "create file ".len() + index;
            (
                &trimmed["create file ".len()..end],
                &trimmed[end + marker.len()..],
            )
        } else if lower.starts_with("write file ") {
            let separator = trimmed.find("::")?;
            (
                &trimmed["write file ".len()..separator],
                trimmed[separator + 2..].trim(),
            )
        } else {
            return None;
        };
        let path = path.trim();
        if path.is_empty() {
            return None;
        }
        Some(Self::FileContents {
            path: path.into(),
            expected: expected.into(),
        })
    }

    pub fn observation_action(&self) -> Action {
        match self {
            Self::FileContents { path, .. } => Action::ReadFile { path: path.clone() },
            Self::FileDigest { path, .. } => Action::HashFile { path: path.clone() },
            Self::FileRange {
                path,
                offset,
                expected,
            } => Action::ReadFileRange {
                path: path.clone(),
                offset: *offset,
                length: expected.len() as u64,
            },
        }
    }
}

pub trait Verifier: Send + Sync {
    fn verify(
        &self,
        criterion: &SuccessCriterion,
        action: &Action,
        observation: &Observation,
    ) -> Verification;
}

fn observation_field(observation: &Observation, field: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(&observation.data)
        .ok()?
        .get(field)?
        .as_str()
        .map(str::to_string)
}

pub struct FileEvidenceVerifier;
impl Verifier for FileEvidenceVerifier {
    fn verify(
        &self,
        criterion: &SuccessCriterion,
        action: &Action,
        observation: &Observation,
    ) -> Verification {
        let bound = action == &criterion.observation_action() && observation.ok;
        let (passed, evidence) = match criterion {
            SuccessCriterion::FileContents { path, expected } => {
                let passed = bound && observation.data == *expected;
                (
                    passed,
                    if passed {
                        format!(
                            "independently verified file '{path}' contains {} expected bytes",
                            expected.len()
                        )
                    } else {
                        format!("file-content evidence failed for '{path}'")
                    },
                )
            }
            SuccessCriterion::FileDigest { path, sha256 } => {
                let observed = bound
                    .then(|| observation_field(observation, "sha256"))
                    .flatten();
                let passed = observed
                    .as_deref()
                    .is_some_and(|digest| digest.eq_ignore_ascii_case(sha256));
                (
                    passed,
                    if passed {
                        format!("independently verified file '{path}' matches digest {sha256}")
                    } else {
                        format!("file-digest evidence failed for '{path}'")
                    },
                )
            }
            SuccessCriterion::FileRange {
                path,
                offset,
                expected,
            } => {
                let observed = bound
                    .then(|| observation_field(observation, "text"))
                    .flatten();
                let passed = observed.as_deref() == Some(expected.as_str());
                (
                    passed,
                    if passed {
                        format!(
                            "independently verified file '{path}' range at {offset} contains {} expected bytes",
                            expected.len()
                        )
                    } else {
                        format!("file-range evidence failed for '{path}' at {offset}")
                    },
                )
            }
        };
        Verification { passed, evidence }
    }
}

/// Historical name retained for existing callers.
pub type FileContentsVerifier = FileEvidenceVerifier;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_evidence_to_the_requested_path_and_successful_observation() {
        let criterion =
            SuccessCriterion::from_objective("write file correct.txt :: expected").unwrap();
        let mut obs = Observation {
            ok: true,
            summary: "untrusted summary".into(),
            data: "expected".into(),
        };
        assert!(
            FileEvidenceVerifier
                .verify(&criterion, &criterion.observation_action(), &obs)
                .passed
        );
        assert!(
            !FileEvidenceVerifier
                .verify(
                    &criterion,
                    &Action::ReadFile {
                        path: "wrong.txt".into()
                    },
                    &obs
                )
                .passed
        );
        obs.ok = false;
        assert!(
            !FileEvidenceVerifier
                .verify(&criterion, &criterion.observation_action(), &obs)
                .passed
        );
        obs.ok = true;
        obs.data = "different".into();
        assert!(
            !FileEvidenceVerifier
                .verify(&criterion, &criterion.observation_action(), &obs)
                .passed
        );
    }

    #[test]
    fn digest_and_range_criteria_bind_evidence_to_typed_observations() {
        let digest = SuccessCriterion::FileDigest {
            path: "data".into(),
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
        };
        let obs = Observation {
            ok: true,
            summary: "completed hash_file:data".into(),
            data: r#"{"algorithm":"sha256","sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad","bytes":3}"#.into(),
        };
        assert!(
            FileEvidenceVerifier
                .verify(&digest, &digest.observation_action(), &obs)
                .passed
        );
        let wrong_digest = SuccessCriterion::FileDigest {
            path: "data".into(),
            sha256: "0".repeat(64),
        };
        assert!(
            !FileEvidenceVerifier
                .verify(&wrong_digest, &wrong_digest.observation_action(), &obs)
                .passed
        );
        assert!(
            !FileEvidenceVerifier
                .verify(
                    &digest,
                    &Action::ReadFile {
                        path: "data".into()
                    },
                    &obs
                )
                .passed
        );

        let range = SuccessCriterion::FileRange {
            path: "data".into(),
            offset: 3,
            expected: "tail".into(),
        };
        assert_eq!(
            range.observation_action(),
            Action::ReadFileRange {
                path: "data".into(),
                offset: 3,
                length: 4,
            }
        );
        let range_obs = Observation {
            ok: true,
            summary: "completed read_range:data:3:4".into(),
            data: r#"{"offset":3,"length":4,"file_size":7,"eof":true,"text":"tail"}"#.into(),
        };
        assert!(
            FileEvidenceVerifier
                .verify(&range, &range.observation_action(), &range_obs)
                .passed
        );
        let stale = Observation {
            ok: true,
            summary: "completed read_range:data:3:4".into(),
            data: r#"{"offset":3,"length":4,"file_size":7,"eof":true,"text":"XXXX"}"#.into(),
        };
        assert!(
            !FileEvidenceVerifier
                .verify(&range, &range.observation_action(), &stale)
                .passed
        );
        let failed = Observation {
            ok: false,
            summary: "failed".into(),
            data: range_obs.data.clone(),
        };
        assert!(
            !FileEvidenceVerifier
                .verify(&range, &range.observation_action(), &failed)
                .passed
        );
    }

    #[test]
    fn preserves_unicode_and_rejects_unsupported_or_empty_claims() {
        let criterion =
            SuccessCriterion::from_objective("create file 日本語.txt with content Tiếng Việt")
                .unwrap();
        assert_eq!(
            criterion,
            SuccessCriterion::FileContents {
                path: "日本語.txt".into(),
                expected: "Tiếng Việt".into()
            }
        );
        for text in ["invent a result", "write file :: contents", "create file x"] {
            assert!(SuccessCriterion::from_objective(text).is_none());
        }
    }
}
