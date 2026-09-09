use crate::event_store::EventStore;
use crate::model::Model;
use crate::permissions::{PermissionDecision, PermissionPolicy};
use crate::plan::{Lifecycle, PlanState};
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
    /// Durable goals, task graph, lifecycle, and budget amendments. Absent
    /// in legacy checkpoints, which decode to an empty active plan.
    #[serde(default)]
    pub plan: PlanState,
    /// Wall-clock origin in unix millis. Set once at creation and never
    /// reset by resume, so time budgets survive restarts. Legacy `None`
    /// backfills on first drive and is documented as a fresh clock.
    #[serde(default)]
    pub started_at_ms: Option<u64>,
    #[serde(default)]
    pub used_tokens: u64,
    #[serde(default)]
    pub used_cost_usd: f64,
    #[serde(default)]
    pub resource_limits: ResourceLimits,
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

/// Cumulative resource bounds. `None` disables that dimension;
/// `max_consecutive_failures: 0` disables the failure trip.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ResourceLimits {
    pub wall_clock_secs: Option<u64>,
    pub token_limit: Option<u64>,
    pub cost_limit_usd: Option<f64>,
    pub max_consecutive_failures: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            wall_clock_secs: None,
            token_limit: None,
            cost_limit_usd: None,
            max_consecutive_failures: 3,
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Process-local shutdown switch. The flag lives outside the checkpoint:
/// requesting shutdown stops the loop after persisting progress, and a
/// later resume continues. Clone it before driving.
#[derive(Clone, Debug, Default)]
pub struct ShutdownHandle {
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl ShutdownHandle {
    pub fn request(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_requested(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::SeqCst)
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
    shutdown: ShutdownHandle,
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
            shutdown: ShutdownHandle::default(),
        }
    }

    /// Process-local graceful-shutdown switch. When requested, the drive
    /// loop checkpoints progress and stops without settling an outcome, so
    /// a later resume continues where it left off.
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        self.shutdown.clone()
    }

    /// Applies only to new runs; resume always uses persisted accounting.
    pub fn with_max_tool_calls(mut self, limit: u64) -> Self {
        self.max_tool_calls = limit;
        self
    }

    /// Bound on cognitive steps for a new run. The persisted `max_steps`
    /// wins on resume; this never raises a resumed run's stored bound.
    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = max_steps;
        self
    }

    /// Heartbeat cadence in cognitive steps. Heartbeats carry the current
    /// budget snapshot for watchdogs and run inspection.
    pub const HEARTBEAT_EVERY: usize = 10;

    /// Accrue the model's metered spend into the checkpoint and enforce
    /// token/cost limits. Spend already happened, so exhaustion fails the
    /// run closed rather than rewinding it.
    fn accrue_usage(&mut self, state: &mut RunState) -> io::Result<Option<RunOutcome>> {
        let usage = self.model.usage();
        // High-water mark: providers report cumulative spend.
        let tokens_before = state.used_tokens;
        let cost_before = state.used_cost_usd;
        if usage.total_tokens() > state.used_tokens {
            state.used_tokens = usage.total_tokens();
        }
        if usage.cost_usd > state.used_cost_usd {
            state.used_cost_usd = usage.cost_usd;
        }
        if state.used_tokens > tokens_before || state.used_cost_usd > cost_before {
            self.events.append(
                "UsageRecorded",
                &serde_json::json!({
                    "prompt_tokens": usage.prompt_tokens,
                    "completion_tokens": usage.completion_tokens,
                    "cost_usd": usage.cost_usd,
                    "total_tokens": state.used_tokens,
                })
                .to_string(),
            )?;
        }
        if let Some(limit) = state.resource_limits.token_limit
            && state.used_tokens > limit
        {
            let reason = format!("token budget exhausted: {} of {limit}", state.used_tokens);
            self.events.append("TokenExhausted", &reason)?;
            self.events.append("GoalFailed", &reason)?;
            return Ok(Some(RunOutcome::Failed(reason)));
        }
        if let Some(limit) = state.resource_limits.cost_limit_usd
            && state.used_cost_usd > limit
        {
            let reason = format!(
                "cost budget exhausted: ${:.6} of ${limit:.6}",
                state.used_cost_usd
            );
            self.events.append("CostExhausted", &reason)?;
            self.events.append("GoalFailed", &reason)?;
            return Ok(Some(RunOutcome::Failed(reason)));
        }
        Ok(None)
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
        self.run_with_plan(objective, PlanState::default(), criterion)
    }

    /// Start a run with a durable goal/task plan attached. The plan is
    /// checkpointed before the first step, so restarts and replans keep the
    /// top-level goal.
    pub fn run_with_plan(
        &mut self,
        objective: Objective,
        plan: PlanState,
        criterion: Option<SuccessCriterion>,
    ) -> io::Result<RunOutcome> {
        let state = self.begin(objective, plan, criterion)?;
        self.events.append("AgentStarted", self.model.name())?;
        self.drive(state)
    }

    /// Create a run without driving it. External schedulers use this to own
    /// the orchestration loop while the runtime owns durable plan state.
    pub fn create_run(
        &mut self,
        objective: Objective,
        plan: PlanState,
        criterion: Option<SuccessCriterion>,
    ) -> io::Result<()> {
        self.begin(objective, plan, criterion)?;
        Ok(())
    }

    fn begin(
        &mut self,
        objective: Objective,
        plan: PlanState,
        criterion: Option<SuccessCriterion>,
    ) -> io::Result<RunState> {
        if self.events.load()?.is_some() {
            return Err(io::Error::other("store already contains a run; use resume"));
        }
        let announce_plan = !plan.goals.is_empty() || !plan.tasks.is_empty();
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
            plan,
            started_at_ms: Some(now_ms()),
            used_tokens: 0,
            used_cost_usd: 0.0,
            resource_limits: ResourceLimits::default(),
        };
        self.events.checkpoint(&state)?;
        let objective = &state.objective;
        self.events.append(
            "GoalCreated",
            &format!("{}:{}", objective.id, objective.text),
        )?;
        if announce_plan {
            self.events.append(
                "PlanCreated",
                &format!(
                    "goals={} tasks={}",
                    state.plan.goals.len(),
                    state.plan.tasks.len()
                ),
            )?;
        }
        Ok(state)
    }

    fn load_live(&mut self) -> io::Result<RunState> {
        let state = self
            .events
            .load()?
            .ok_or_else(|| io::Error::other("no checkpoint to resume"))?;
        if state.version != 1
            || state.workspace != std::fs::canonicalize(self.policy.workspace_root())?
        {
            return Err(io::Error::other("checkpoint version or workspace mismatch"));
        }
        if state.outcome.is_some() {
            return Err(io::Error::other(
                "run is terminal; no further transitions allowed",
            ));
        }
        Ok(state)
    }

    /// Read the durable plan without changing it.
    pub fn plan_snapshot(&mut self) -> io::Result<PlanState> {
        Ok(self.load_live()?.plan)
    }

    /// Apply a plan transition durably: the mutation commits to the
    /// checkpoint before returning, and a `PlanRevised` audit event records
    /// what changed. Terminal runs refuse revision.
    pub fn revise_plan(
        &mut self,
        what: &str,
        edit: impl FnOnce(&mut PlanState) -> Result<(), String>,
    ) -> io::Result<()> {
        let mut state = self.load_live()?;
        edit(&mut state.plan).map_err(io::Error::other)?;
        self.events.checkpoint(&state)?;
        self.events.append("PlanRevised", what)?;
        Ok(())
    }

    /// Move the lifecycle durably. Paused runs refuse to drive until
    /// resumed; cancelled and completed runs are terminal.
    pub fn set_lifecycle(&mut self, lifecycle: Lifecycle) -> io::Result<()> {
        let mut state = self.load_live()?;
        state.plan.lifecycle = lifecycle.clone();
        self.events.checkpoint(&state)?;
        self.events
            .append("LifecycleChanged", &format!("lifecycle={lifecycle:?}"))?;
        Ok(())
    }

    /// Finish a driver-owned run. Requires every planned task to be
    /// succeeded or abandoned; the top-level goal record stays attached to
    /// the terminal checkpoint.
    pub fn complete_run(&mut self, summary: impl Into<String>) -> io::Result<RunOutcome> {
        let mut state = self.load_live()?;
        if !state.plan.goals_complete() {
            return Err(io::Error::other(
                "plan goals are not complete; repair or abandon open tasks first",
            ));
        }
        let outcome = RunOutcome::Completed(summary.into());
        state.plan.lifecycle = Lifecycle::Completed;
        state.outcome = Some(outcome.clone());
        self.events.checkpoint(&state)?;
        self.events.append("GoalCompleted", "plan goals complete")?;
        Ok(outcome)
    }

    /// Cancel a live run. Terminal: the failure records the operator reason.
    pub fn cancel_run(&mut self, reason: impl Into<String>) -> io::Result<RunOutcome> {
        let mut state = self.load_live()?;
        let outcome = RunOutcome::Failed(reason.into());
        state.plan.lifecycle = Lifecycle::Cancelled;
        state.outcome = Some(outcome.clone());
        self.events.checkpoint(&state)?;
        self.events.append("GoalFailed", "run cancelled")?;
        Ok(outcome)
    }

    /// Controlled budget amendment. The previous limit, the new limit, and
    /// the reason are recorded in the plan and in a `BudgetAmended` event;
    /// limits are never silently reset or replenished.
    pub fn amend_tool_budget(
        &mut self,
        new_limit: u64,
        reason: impl Into<String>,
    ) -> io::Result<()> {
        let mut state = self.load_live()?;
        let budget = state.tool_budget.as_mut().ok_or_else(|| {
            io::Error::other(
                "legacy checkpoint lacks tool accounting; automatic continuation refused",
            )
        })?;
        let reason = reason.into();
        state.plan.amendments.push(crate::plan::BudgetAmendment {
            previous_limit: budget.limit,
            new_limit,
            reason: reason.clone(),
        });
        budget.limit = new_limit;
        self.events.checkpoint(&state)?;
        self.events.append("BudgetAmended", &reason)?;
        Ok(())
    }

    /// Set (or repair) the explicit success claim on a live run. Audited;
    /// used when resuming checkpoints that predate the claim.
    pub fn set_success_criterion(&mut self, criterion: SuccessCriterion) -> io::Result<()> {
        let mut state = self.load_live()?;
        state.success_criterion = Some(criterion);
        self.events.checkpoint(&state)?;
        self.events
            .append("SuccessCriterionRevised", "explicit claim set")?;
        Ok(())
    }

    /// Replace the whole resource-limits set with an audited reason. Like
    /// tool-call amendments, this never silently resets.
    pub fn amend_resource_limits(
        &mut self,
        limits: ResourceLimits,
        reason: impl Into<String>,
    ) -> io::Result<()> {
        let mut state = self.load_live()?;
        state.resource_limits = limits;
        self.events.checkpoint(&state)?;
        self.events
            .append("ResourceLimitsAmended", &reason.into())?;
        Ok(())
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
        if matches!(result, RunOutcome::Completed(_)) {
            state.plan.lifecycle = Lifecycle::Completed;
        }
        state.outcome = Some(result.clone());
        self.events.checkpoint(&state)?;
        Ok(result)
    }

    fn drive_inner(&mut self, state: &mut RunState) -> io::Result<RunOutcome> {
        let objective = state.objective.clone();
        match state.plan.lifecycle {
            Lifecycle::Paused => {
                return Err(io::Error::other("run is paused; resume it before driving"));
            }
            Lifecycle::Cancelled => return Ok(RunOutcome::Failed("run was cancelled".into())),
            Lifecycle::Completed => {
                return Ok(RunOutcome::Failed(
                    "run is already completed; no further steps".into(),
                ));
            }
            Lifecycle::Active => {}
        }

        if state.started_at_ms.is_none() {
            state.started_at_ms = Some(now_ms());
        }
        while state.steps < state.max_steps {
            if self.shutdown.is_requested() {
                self.events.checkpoint(state)?;
                self.events
                    .append("ShutdownRequested", &state.objective.id)?;
                return Err(io::Error::other("shutdown requested; resume to continue"));
            }
            if let Some(limit) = state.resource_limits.wall_clock_secs {
                let elapsed = now_ms().saturating_sub(state.started_at_ms.unwrap_or(0)) / 1000;
                if elapsed > limit {
                    let reason =
                        format!("wall-clock budget exhausted after {elapsed}s of {limit}s");
                    self.events.append("WallClockExhausted", &reason)?;
                    self.events.append("GoalFailed", &reason)?;
                    return Ok(RunOutcome::Failed(reason));
                }
            }
            let trailing_failures = state
                .history
                .iter()
                .rev()
                .take_while(|(_, observation)| !observation.ok)
                .count() as u64;
            let trip = state.resource_limits.max_consecutive_failures;
            if trip > 0 && trailing_failures >= trip {
                let reason =
                    format!("repeated errors: {trailing_failures} consecutive tool failures");
                self.events.append("RepeatedFailures", &reason)?;
                self.events.append("GoalFailed", &reason)?;
                return Ok(RunOutcome::Failed(reason));
            }
            let step = state.steps;
            state.steps += 1;
            self.events.checkpoint(state)?;
            self.events.append(
                "CognitiveStep",
                &format!("step={step};phase=orient_recall_plan"),
            )?;
            if step > 0 && step.is_multiple_of(Self::HEARTBEAT_EVERY) {
                self.events.append(
                    "Heartbeat",
                    &format!(
                        "step={step};tools={}/{};tokens={};cost_usd={:.6}",
                        state
                            .tool_budget
                            .as_ref()
                            .map(|budget| budget.used)
                            .unwrap_or(0),
                        state
                            .tool_budget
                            .as_ref()
                            .map(|budget| budget.limit)
                            .unwrap_or(0),
                        state.used_tokens,
                        state.used_cost_usd,
                    ),
                )?;
            }
            let decision = self.model.decide(&objective, &state.history);
            if let Some(outcome) = self.accrue_usage(state)? {
                return Ok(outcome);
            }
            match decision {
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
