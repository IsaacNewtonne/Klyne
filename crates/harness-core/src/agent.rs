use crate::event_store::EventStore;
use crate::model::Model;
use crate::permissions::{PermissionDecision, PermissionPolicy};
use crate::tools::ToolRegistry;
use crate::types::{Action, Objective, Observation, StepDecision};
use std::io;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    Completed(String),
    Failed(String),
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
        Self { model, tools, policy, events, max_steps: 8 }
    }

    fn execute_action(
        &mut self,
        action: Action,
        phase: &str,
        history: &mut Vec<(Action, Observation)>,
    ) -> io::Result<Option<RunOutcome>> {
        self.events.append("PlanUpdated", &format!("phase={phase};action={action}"))?;
        match self.policy.check(&action) {
            PermissionDecision::Allow => { self.events.append("PermissionGranted", &action.to_string())?; }
            PermissionDecision::Deny(reason) => {
                self.events.append("PermissionDenied", &reason)?;
                return Ok(Some(RunOutcome::Failed(reason)));
            }
            PermissionDecision::Ask(reason) => {
                self.events.append("PermissionRequested", &reason)?;
                return Ok(Some(RunOutcome::Failed(format!("approval required: {reason}"))));
            }
        }
        self.events.append("ToolCalled", &action.to_string())?;
        let obs = self.tools.execute(&action, &self.policy);
        self.events.append(
            if obs.ok { "ToolCompleted" } else { "ToolFailed" },
            &format!("{};{}", obs.summary, obs.data),
        )?;
        history.push((action, obs));
        Ok(None)
    }

    pub fn run(&mut self, objective: Objective) -> io::Result<RunOutcome> {
        self.events.append("GoalCreated", &format!("{}:{}", objective.id, objective.text))?;
        self.events.append("AgentStarted", self.model.name())?;
        let mut history: Vec<(Action, Observation)> = Vec::new();

        for step in 0..self.max_steps {
            self.events.append("CognitiveStep", &format!("step={step};phase=orient_recall_plan"))?;
            match self.model.decide(&objective, &history) {
                StepDecision::Act(action) => {
                    if let Some(outcome) = self.execute_action(action, "act", &mut history)? {
                        return Ok(outcome);
                    }
                }
                StepDecision::Verify(action) => {
                    if let Some(outcome) = self.execute_action(action, "verify", &mut history)? {
                        return Ok(outcome);
                    }
                }
                StepDecision::Complete(summary) => {
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
        let reason = format!("runaway-loop guard: exceeded {} cognitive steps", self.max_steps);
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
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("harness-test-{suffix}"));
        fs::create_dir_all(&root).unwrap();
        let events = FileEventStore::open(root.join("events.log")).unwrap();
        let policy = PermissionPolicy::milestone_default(&root);
        let mut runtime = AgentRuntime::new(HeuristicModel, ToolRegistry::milestone_default(), policy, events);
        let outcome = runtime.run(Objective::new("create file result.txt with content verified")).unwrap();
        assert!(matches!(outcome, RunOutcome::Completed(_)));
        assert_eq!(fs::read_to_string(root.join("result.txt")).unwrap(), "verified");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn blocks_path_traversal() {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("harness-test-{suffix}"));
        fs::create_dir_all(&root).unwrap();
        let events = FileEventStore::open(root.join("events.log")).unwrap();
        let policy = PermissionPolicy::milestone_default(&root);
        let mut runtime = AgentRuntime::new(HeuristicModel, ToolRegistry::milestone_default(), policy, events);
        let outcome = runtime.run(Objective::new("create file ../escape.txt with content blocked")).unwrap();
        assert!(matches!(outcome, RunOutcome::Failed(_)));
        fs::remove_dir_all(root).unwrap();
    }
}
