use crate::event_store::EventStore;
use crate::model::Model;
use crate::permissions::{PermissionDecision, PermissionPolicy};
use crate::tools::ToolRegistry;
use crate::types::{Action, Objective, Observation, StepDecision};
use crate::verification::{FileEvidenceVerifier, SuccessCriterion, Verifier};
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
    #[serde(default)]
    pub tool_budget: Option<ToolBudget>,
    /// Explicit success claim for runs whose objective grammar the CLI adapter
    /// cannot parse (e.g. digest/range repair goals). `None` derives the claim
    /// from the objective text, preserving legacy behavior.
    #[serde(default)]
    pub success_criterion: Option<SuccessCriterion>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ToolBudget {
    pub used: u64,
    pub limit: u64,
}

impl Default for ToolBudget {
    fn default() -> Self {
        Self { used: 0, limit: 32 }
    }
}

impl RunState {
    /// Stable within a run, including across restarts and reconciliation attempts.
    pub fn next_action_id(&self) -> String {
        format!("{}/action/{}", self.objective.id, self.history.len())
    }
}

pub struct AgentRuntime<M: Model, E: EventStore> {
    model: M,
    tools: ToolRegistry,
    policy: PermissionPolicy,
    events: E,
    max_steps: usize,
    max_tool_calls: u64,
}

impl<M: Model, E: EventStore> AgentRuntime<M, E> {
    pub fn new(model: M, tools: ToolRegistry, policy: PermissionPolicy, events: E) -> Self {
        Self {
            model,
            tools,
            policy,
            events,
            max_steps: 8,
            max_tool_calls: ToolBudget::default().limit,
        }
    }

    /// Applies only to new runs; resume always uses persisted accounting.
    pub fn with_max_tool_calls(mut self, limit: u64) -> Self {
        self.max_tool_calls = limit;
        self
    }

    fn reserve_tool_call(&mut self, state: &mut RunState) -> io::Result<()> {
        let budget = state.tool_budget.as_mut().ok_or_else(|| {
            io::Error::other(
                "legacy checkpoint lacks tool accounting; automatic continuation refused",
            )
        })?;
        if budget.used >= budget.limit {
            self.events
                .append("BudgetExhausted", "tool-call budget exhausted")?;
            return Err(io::Error::other("tool-call budget exhausted"));
        }
        budget.used += 1;
        // Reserve before any invocation, even a failing reconciliation read.
        // A failed commit prevents invocation; a crash can consume unused credit.
        self.events.checkpoint(state)
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
        state.pending = Some(action.clone());
        self.reserve_tool_call(state)?;
        self.events.append("ToolCalled", &action.to_string())?;
        self.action_event("ActionPrepared", state, &action, None)?;
        let obs = self.tools.execute(&action, &self.policy);
        self.action_event("ActionObserved", state, &action, Some(&obs))?;
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
        self.run_with_criterion(objective, None)
    }

    /// Start a run with an explicit machine-checked success claim instead of
    /// deriving one from the objective text. The claim persists in the
    /// checkpoint and survives resume; budgets, permissions, and independent
    /// verification behave identically to [`AgentRuntime::run`].
    pub fn run_with_criterion(
        &mut self,
        objective: Objective,
        criterion: Option<SuccessCriterion>,
    ) -> io::Result<RunOutcome> {
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
            tool_budget: Some(ToolBudget {
                used: 0,
                limit: self.max_tool_calls,
            }),
            success_criterion: criterion,
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
        self.resume_with_reconciliation(false)
    }

    /// Inspect interrupted file actions without repeating a write. A matching
    /// postcondition proves current state, not which process produced it.
    pub fn reconcile_and_resume(&mut self) -> io::Result<RunOutcome> {
        self.resume_with_reconciliation(true)
    }

    fn action_event(
        &mut self,
        kind: &str,
        state: &RunState,
        action: &Action,
        observation: Option<&Observation>,
    ) -> io::Result<()> {
        let detail = serde_json::json!({"version":1, "action_id":state.next_action_id(), "action":action, "observation":observation});
        self.events.append(kind, &detail.to_string())?;
        Ok(())
    }

    fn reconcile_pending(&mut self, state: &mut RunState) -> io::Result<()> {
        let Some(action) = state.pending.clone() else {
            return Ok(());
        };
        self.action_event("ReconciliationStarted", state, &action, None)?;
        // Read-only actions recover with a fresh observation; interrupted
        // writes are confirmed by postcondition, never replayed. Patches,
        // shell commands, and terminal markers stay blocked: their effects
        // cannot be distinguished as before/after/conflicting from outside.
        let read = match &action {
            Action::WriteFile { path, .. } | Action::ReadFile { path } => {
                Action::ReadFile { path: path.clone() }
            }
            Action::ReadFileRange { .. } | Action::HashFile { .. } | Action::SearchFile { .. } => {
                action.clone()
            }
            _ => {
                return Err(io::Error::other(
                    "reconciliation is unsupported for this action; no replay performed",
                ));
            }
        };
        for request in [&action, &read] {
            if let PermissionDecision::Deny(reason) | PermissionDecision::Ask(reason) =
                self.policy.check(request)
            {
                self.action_event("ReconciliationDenied", state, &action, None)?;
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, reason));
            }
        }
        self.reserve_tool_call(state)?;
        let observed = self.tools.execute(&read, &self.policy);
        self.action_event("ReconciliationObserved", state, &read, Some(&observed))?;
        if !observed.ok
            || matches!(&action, Action::WriteFile { contents, .. } if *contents != observed.data)
        {
            self.action_event("ReconciliationBlocked", state, &action, None)?;
            return Err(io::Error::other(
                "reconciliation could not establish the postcondition; pending action preserved",
            ));
        }
        let result = match &action {
            Action::WriteFile { contents, .. } => Observation {
                ok: true,
                summary: "write postcondition observed; write was not repeated".into(),
                data: contents.len().to_string(),
            },
            _ => observed,
        };
        self.action_event("ActionReconciled", state, &action, Some(&result))?;
        state.history.push((action, result));
        state.pending = None;
        self.events.checkpoint(state)
    }

    fn resume_with_reconciliation(&mut self, reconcile: bool) -> io::Result<RunOutcome> {
        let mut state = self
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
        if state.tool_budget.is_none() {
            return Err(io::Error::other(
                "legacy checkpoint lacks tool accounting; automatic continuation refused",
            ));
        }
        if reconcile {
            self.reconcile_pending(&mut state)?;
        }
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
                    let criterion = state
                        .success_criterion
                        .clone()
                        .or_else(|| SuccessCriterion::from_objective(&objective.text));
                    let Some(criterion) = criterion else {
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
                    let verification = FileEvidenceVerifier.verify(&criterion, action, observation);
                    if !verification.passed {
                        let reason = "independent file verification failed";
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
