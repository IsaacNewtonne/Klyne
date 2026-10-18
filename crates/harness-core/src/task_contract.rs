//! Caller-owned acceptance criteria; model review cannot replace these checks.
use crate::{
    PermissionPolicy, ToolRegistry,
    artifact_requirements::CheckOutcome,
    verification::{FileEvidenceVerifier, SuccessCriterion, Verifier},
};
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskContract {
    pub goal: String,
    pub criteria: Vec<SuccessCriterion>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcceptanceCheck {
    pub criterion: usize,
    pub passed: bool,
    pub evidence: String,
    /// Absent on legacy checks; never infer new evidence for old conversations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CheckOutcome>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    Verified,
    Unmet,
    Reviewed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskResult {
    pub outcome: TaskOutcome,
    pub checks: Vec<AcceptanceCheck>,
}
impl TaskContract {
    pub fn from_goal(goal: &str) -> Option<Self> {
        SuccessCriterion::from_objective(goal).map(|criterion| Self {
            goal: goal.into(),
            criteria: vec![criterion],
        })
    }
    pub fn validate(&self, goal: &str) -> io::Result<()> {
        if self.goal != goal
            || self.criteria.is_empty()
            || self.criteria.len() > 16
            || serde_json::to_vec(self)?.len() > 16384
        {
            return Err(io::Error::other(
                "Contract must match the instruction and contain 1–16 bounded acceptance criteria",
            ));
        }
        for criterion in &self.criteria {
            if let SuccessCriterion::ArtifactRequirements(requirements) = criterion {
                requirements.validate()?;
            }
        }
        Ok(())
    }
    pub fn validate_scope(&self, policy: &PermissionPolicy) -> io::Result<()> {
        for criterion in &self.criteria {
            if let SuccessCriterion::ArtifactRequirements(requirements) = criterion {
                requirements.validate_scope(policy)?;
            }
        }
        Ok(())
    }
    pub fn verify(&self, policy: &PermissionPolicy) -> TaskResult {
        let tools = ToolRegistry::milestone_default();
        let checks: Vec<_> = self
            .criteria
            .iter()
            .enumerate()
            .map(|(index, criterion)| {
                if let SuccessCriterion::ArtifactRequirements(requirements) = criterion {
                    let (outcome, evidence) = match requirements.validate_scope(policy) {
                        Err(error) => (CheckOutcome::Failed, error.to_string()),
                        Ok(()) => {
                            let observed = tools.execute(&criterion.observation_action(), policy);
                            if observed.ok {
                                requirements.check(&observed.data)
                            } else {
                                (
                                    CheckOutcome::Inconclusive,
                                    format!(
                                        "Could not inspect report '{}': {}: {}",
                                        requirements.path, observed.summary, observed.data
                                    ),
                                )
                            }
                        }
                    };
                    return AcceptanceCheck {
                        criterion: index,
                        passed: outcome == CheckOutcome::Passed,
                        evidence,
                        outcome: Some(outcome),
                    };
                }
                let action = criterion.observation_action();
                let observed = tools.execute(&action, policy);
                let verification = FileEvidenceVerifier.verify(criterion, &action, &observed);
                AcceptanceCheck {
                    criterion: index,
                    passed: verification.passed,
                    evidence: verification.evidence,
                    outcome: None,
                }
            })
            .collect();
        let passed = !checks.is_empty() && checks.iter().all(|c| c.passed);
        TaskResult {
            outcome: if passed {
                TaskOutcome::Verified
            } else {
                TaskOutcome::Unmet
            },
            checks,
        }
    }
}
