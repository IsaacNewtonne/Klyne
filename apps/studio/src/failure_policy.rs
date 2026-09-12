//! Host-owned recovery decisions. Model error prose never authorizes a retry.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    UncertainEffect,
    Stopped,
    BudgetExhausted,
    Precondition,
    RecoveryExhausted,
    MissingInput,
    Unclassified,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NextStep {
    Reconcile,
    Resume,
    ObserveAndReplan,
    RequestInput,
    Inspect,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FailureDecision {
    pub kind: FailureKind,
    pub next_step: NextStep,
    pub automatic: bool,
    pub retry_allowed: bool,
    pub guidance: String,
}
pub fn decide(kind: FailureKind) -> FailureDecision {
    let (next_step, automatic, guidance) = match kind {
        FailureKind::UncertainEffect => (
            NextStep::Reconcile,
            false,
            "Inspect the destination and reconcile the pending action before resuming. Do not repeat it through another route.",
        ),
        FailureKind::Stopped => (
            NextStep::Resume,
            false,
            "Work is saved. Resume when ready; completed steps will be skipped.",
        ),
        FailureKind::BudgetExhausted => (
            NextStep::Resume,
            false,
            "The execution budget ended. Review the saved progress before resuming.",
        ),
        FailureKind::Precondition => (
            NextStep::ObserveAndReplan,
            true,
            "Input was not applied. Use the fresh observation to choose a corrected action; do not repeat the unchanged failure.",
        ),
        FailureKind::RecoveryExhausted => (
            NextStep::Inspect,
            false,
            "Three recovery attempts made no progress. Inspect the blocker before continuing.",
        ),
        FailureKind::MissingInput => (
            NextStep::RequestInput,
            false,
            "Answer the question to continue the unfinished step.",
        ),
        FailureKind::Unclassified => (
            NextStep::Inspect,
            false,
            "Inspect the recorded error and evidence. No automatic retry or route switch is authorized by an unclassified failure.",
        ),
    };
    FailureDecision {
        kind,
        next_step,
        automatic,
        retry_allowed: false,
        guidance: guidance.into(),
    }
}
pub fn terminal(pending: bool, stopped: bool, budget_exhausted: bool) -> FailureDecision {
    decide(if pending {
        FailureKind::UncertainEffect
    } else if stopped {
        FailureKind::Stopped
    } else if budget_exhausted {
        FailureKind::BudgetExhausted
    } else {
        FailureKind::Unclassified
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uncertain_effect_overrides_stop_and_budget() {
        let decision = terminal(true, true, true);
        assert_eq!(decision.next_step, NextStep::Reconcile);
        assert!(!decision.automatic && !decision.retry_allowed);
        assert_eq!(terminal(false, true, true).kind, FailureKind::Stopped);
        assert_eq!(
            terminal(false, false, true).kind,
            FailureKind::BudgetExhausted
        );
    }
    #[test]
    fn only_known_preconditions_continue_automatically() {
        for kind in [
            FailureKind::UncertainEffect,
            FailureKind::Stopped,
            FailureKind::BudgetExhausted,
            FailureKind::RecoveryExhausted,
            FailureKind::MissingInput,
            FailureKind::Unclassified,
        ] {
            assert!(!decide(kind).automatic);
        }
        let decision = decide(FailureKind::Precondition);
        assert!(decision.automatic);
        assert!(!decision.retry_allowed);
    }
}
