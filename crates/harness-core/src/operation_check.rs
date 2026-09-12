//! Predicates over fresh adapter-owned observations, never model assertions.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Predicate {
    Equals {
        pointer: String,
        expected: Value,
    },
    UniqueRow {
        pointer: String,
        key: String,
        identity: Value,
        field: String,
        expected: Value,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationCheck {
    pub predicates: Vec<Predicate>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    Verified,
    Unmet,
    Unknown,
}

#[derive(Debug, Serialize)]
pub struct CheckResult {
    pub outcome: CheckOutcome,
    pub retry_safe: bool,
    pub reason: &'static str,
}

impl OperationCheck {
    pub fn evaluate(&self, observation: &Value) -> CheckResult {
        let mut missing = self.predicates.is_empty() || self.predicates.len() > 16;
        let mut mismatch = false;
        for predicate in self.predicates.iter().take(16) {
            let (actual, expected) = match predicate {
                Predicate::Equals { pointer, expected } => (observation.pointer(pointer), expected),
                Predicate::UniqueRow {
                    pointer,
                    key,
                    identity,
                    field,
                    expected,
                } => {
                    let rows = observation.pointer(pointer).and_then(Value::as_array);
                    let found: Vec<_> = rows
                        .into_iter()
                        .flatten()
                        .filter(|row| row.get(key) == Some(identity))
                        .take(2)
                        .collect();
                    (
                        if found.len() == 1 {
                            found[0].get(field)
                        } else {
                            None
                        },
                        expected,
                    )
                }
            };
            match actual {
                Some(value) => mismatch |= value != expected,
                None => missing = true,
            }
        }
        CheckResult {
            outcome: if missing {
                CheckOutcome::Unknown
            } else if mismatch {
                CheckOutcome::Unmet
            } else {
                CheckOutcome::Verified
            },
            // Current state cannot establish whether an earlier effect occurred.
            retry_safe: false,
            reason: if missing {
                "Required evidence missing or ambiguous"
            } else if mismatch {
                "Observed state does not satisfy the postcondition"
            } else {
                "Fresh adapter observation satisfies the postcondition"
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn strict_evidence_distinguishes_missing_mismatch_and_success() {
        let check = OperationCheck {
            predicates: vec![Predicate::Equals {
                pointer: "/saved".into(),
                expected: json!(true),
            }],
        };
        assert_eq!(
            check.evaluate(&json!({"saved":true})).outcome,
            CheckOutcome::Verified
        );
        assert_eq!(
            check.evaluate(&json!({"saved":"true"})).outcome,
            CheckOutcome::Unmet
        );
        assert_eq!(
            check.evaluate(&json!({"ok":true})).outcome,
            CheckOutcome::Unknown
        );
        assert!(!check.evaluate(&json!({"saved":false})).retry_safe);
        assert_eq!(
            OperationCheck { predicates: vec![] }
                .evaluate(&json!({}))
                .outcome,
            CheckOutcome::Unknown
        );
    }
    #[test]
    fn ambiguous_identity_cannot_verify() {
        let check = OperationCheck {
            predicates: vec![Predicate::UniqueRow {
                pointer: "/rows".into(),
                key: "id".into(),
                identity: json!("doc"),
                field: "saved".into(),
                expected: json!(true),
            }],
        };
        assert_eq!(
            check
                .evaluate(&json!({"rows":[{"id":"doc","saved":true}]}))
                .outcome,
            CheckOutcome::Verified
        );
        assert_eq!(
            check
                .evaluate(&json!({"rows":[{"id":"doc","saved":true},{"id":"doc","saved":true}]}))
                .outcome,
            CheckOutcome::Unknown
        );
    }
}
