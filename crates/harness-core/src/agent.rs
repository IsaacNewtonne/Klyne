use crate::event_store::EventStore;
use crate::model::Model;
use crate::permissions::{PermissionDecision, PermissionPolicy};
use crate::tools::ToolRegistry;
use crate::types::{Action, Objective, Observation, StepDecision};
use crate::verification::{FileContentsVerifier, SuccessCriterion, Verifier};
use std::io;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RunOutcome {
    Completed(String),
    Failed(String),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RunState {
    pub version: u32,
    pub objective: Objective,
    pub workspace: std::path::PathBuf,
    pub history: Vec<(Action, Observation)>,
    pub steps: usize,
    pub max_steps: usize,
    pub pending: Option<Action>,
    pub outcome: Option<RunOutcome>,
}

pub struct AgentRuntime<M: Model, E: EventStore> {
    model: M,
    tools: ToolRegistry,
    policy: PermissionPolicy,
    events: E,
    max_steps: usize,
}

impl<M: Model, E: EventStore> AgentRuntime<M, E> {
    pub fn new(model: M, tools: ToolRegistry, policy: PermissionPolicy, events: E) -> Self {
        Self {
            model,
            tools,
            policy,
            events,
            max_steps: 8,
        }
    }

    fn execute_action(
        &mut self,
        action: Action,
        phase: &str,
        state: &mut RunState,
    ) -> io::Result<Option<RunOutcome>> {
        self.events
            .append("PlanUpdated", &format!("phase={phase};action={action}"))?;
        match self.policy.check(&action) {
            PermissionDecision::Allow => {
                self.events
                    .append("PermissionGranted", &action.to_string())?;
            }
            PermissionDecision::Deny(reason) => {
                self.events.append("PermissionDenied", &reason)?;
                self.events.append("GoalFailed", &reason)?;
                return Ok(Some(RunOutcome::Failed(reason)));
            }
            PermissionDecision::Ask(reason) => {
                self.events.append("PermissionRequested", &reason)?;
                return Ok(Some(RunOutcome::Failed(format!(
                    "approval required: {reason}"
                ))));
            }
        }
        self.events.append("ToolCalled", &action.to_string())?;
        state.pending = Some(action.clone());
        self.events.checkpoint(state)?;
        let obs = self.tools.execute(&action, &self.policy);
        self.events.append(
            if obs.ok {
                "ToolCompleted"
            } else {
                "ToolFailed"
            },
            &format!("{};{}", obs.summary, obs.data),
        )?;
        state.history.push((action, obs));
        state.pending = None;
        self.events.checkpoint(state)?;
        Ok(None)
    }

    pub fn run(&mut self, objective: Objective) -> io::Result<RunOutcome> {
        if self.events.load()?.is_some() {
            return Err(io::Error::other("store already contains a run; use resume"));
        }
        let state = RunState {
            version: 1,
            objective,
            workspace: std::fs::canonicalize(self.policy.workspace_root())?,
            history: Vec::new(),
            steps: 0,
            max_steps: self.max_steps,
            pending: None,
            outcome: None,
        };
        self.events.checkpoint(&state)?;
        let objective = &state.objective;
        self.events.append(
            "GoalCreated",
            &format!("{}:{}", objective.id, objective.text),
        )?;
        self.events.append("AgentStarted", self.model.name())?;
        self.drive(state)
    }

    pub fn resume(&mut self) -> io::Result<RunOutcome> {
        let state = self
            .events
            .load()?
            .ok_or_else(|| io::Error::other("no checkpoint to resume"))?;
        if state.version != 1
            || state.workspace != std::fs::canonicalize(self.policy.workspace_root())?
        {
            return Err(io::Error::other("checkpoint version or workspace mismatch"));
        }
        if let Some(outcome) = state.outcome {
            return Ok(outcome);
        }
        // Never replay an ambiguous side effect. A future reconciler can inspect
        // preconditions and postconditions before explicitly resolving this state.
        if state.pending.is_some() {
            return Err(io::Error::other(
                "interrupted action has uncertain outcome; reconciliation required",
            ));
        }
        self.events.append("AgentResumed", &state.objective.id)?;
        self.drive(state)
    }

    fn drive(&mut self, mut state: RunState) -> io::Result<RunOutcome> {
        let result = self.drive_inner(&mut state)?;
        state.outcome = Some(result.clone());
        self.events.checkpoint(&state)?;
        Ok(result)
    }

    fn drive_inner(&mut self, state: &mut RunState) -> io::Result<RunOutcome> {
        let objective = state.objective.clone();

        while state.steps < state.max_steps {
            let step = state.steps;
            state.steps += 1;
            self.events.checkpoint(state)?;
            self.events.append(
                "CognitiveStep",
                &format!("step={step};phase=orient_recall_plan"),
            )?;
            match self.model.decide(&objective, &state.history) {
                StepDecision::Act(action) => {
                    if let Some(outcome) = self.execute_action(action, "act", state)? {
                        return Ok(outcome);
                    }
                }
                StepDecision::Verify(action) => {
                    if let Some(outcome) = self.execute_action(action, "verify", state)? {
                        return Ok(outcome);
                    }
                }
                StepDecision::Complete(_) => {
                    self.events.append("VerificationStarted", &objective.id)?;
                    let Some(criterion) = SuccessCriterion::from_objective(&objective.text) else {
                        let reason = "objective has no supported success criterion";
                        self.events.append("VerificationFailed", reason)?;
                        self.events.append("GoalFailed", reason)?;
                        return Ok(RunOutcome::Failed(reason.into()));
                    };
                    if let Some(outcome) = self.execute_action(
                        criterion.observation_action(),
                        "independent_verification",
                        state,
                    )? {
                        return Ok(outcome);
                    }
                    let (action, observation) = state
                        .history
                        .last()
                        .ok_or_else(|| io::Error::other("missing verification observation"))?;
                    let verification = FileContentsVerifier.verify(&criterion, action, observation);
                    if !verification.passed {
                        let reason = "independent file-content verification failed";
                        self.events.append("VerificationFailed", reason)?;
                        self.events.append("GoalFailed", reason)?;
                        return Ok(RunOutcome::Failed(reason.into()));
                    }
                    let summary = verification.evidence;
                    self.events.append("VerificationPassed", &summary)?;
                    self.events.append("GoalCompleted", &summary)?;
                    return Ok(RunOutcome::Completed(summary));
                }
                StepDecision::Fail(reason) => {
                    self.events.append("GoalFailed", &reason)?;
                    return Ok(RunOutcome::Failed(reason));
                }
            }
        }
        let reason = format!(
            "runaway-loop guard: exceeded {} cognitive steps",
            state.max_steps
        );
        self.events.append("GoalFailed", &reason)?;
        Ok(RunOutcome::Failed(reason))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileEventStore, HeuristicModel, ToolRegistry};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn writes_then_reads_then_verifies() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("harness-test-{suffix}"));
        fs::create_dir_all(&root).unwrap();
        let events = FileEventStore::open(root.join("events.log")).unwrap();
        let policy = PermissionPolicy::milestone_default(&root);
        let mut runtime = AgentRuntime::new(
            HeuristicModel,
            ToolRegistry::milestone_default(),
            policy,
            events,
        );
        let outcome = runtime
            .run(Objective::new(
                "create file result.txt with content verified",
            ))
            .unwrap();
        assert!(matches!(outcome, RunOutcome::Completed(_)));
        assert_eq!(
            fs::read_to_string(root.join("result.txt")).unwrap(),
            "verified"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn blocks_path_traversal() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("harness-test-{suffix}"));
        fs::create_dir_all(&root).unwrap();
        let events = FileEventStore::open(root.join("events.log")).unwrap();
        let policy = PermissionPolicy::milestone_default(&root);
        let mut runtime = AgentRuntime::new(
            HeuristicModel,
            ToolRegistry::milestone_default(),
            policy,
            events,
        );
        let outcome = runtime
            .run(Objective::new(
                "create file ../escape.txt with content blocked",
            ))
            .unwrap();
        assert!(matches!(outcome, RunOutcome::Failed(_)));
        fs::remove_dir_all(root).unwrap();
    }
}
