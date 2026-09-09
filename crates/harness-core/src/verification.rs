//! Success criteria belong to the harness protocol, independently of any model.
use crate::types::{Action, Observation, Verification};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuccessCriterion {
    FileContents { path: String, expected: String },
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

pub struct FileContentsVerifier;
impl Verifier for FileContentsVerifier {
    fn verify(
        &self,
        criterion: &SuccessCriterion,
        action: &Action,
        observation: &Observation,
    ) -> Verification {
        let SuccessCriterion::FileContents { path, expected } = criterion;
        let passed = action == &criterion.observation_action()
            && observation.ok
            && observation.data == *expected;
        Verification {
            passed,
            evidence: if passed {
                format!(
                    "independently verified file '{path}' contains {} expected bytes",
                    expected.len()
                )
            } else {
                format!("file-content evidence failed for '{path}'")
            },
        }
    }
}

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
            FileContentsVerifier
                .verify(&criterion, &criterion.observation_action(), &obs)
                .passed
        );
        assert!(
            !FileContentsVerifier
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
            !FileContentsVerifier
                .verify(&criterion, &criterion.observation_action(), &obs)
                .passed
        );
        obs.ok = true;
        obs.data = "different".into();
        assert!(
            !FileContentsVerifier
                .verify(&criterion, &criterion.observation_action(), &obs)
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
