//! Durable goals and plans.
//!
//! A plan is persisted intent, not an executor: goals name the top-level
//! outcomes, tasks form an explicit dependency graph, and every transition
//! (including budget amendments and replans) is recorded rather than applied
//! silently. The runtime checkpoints this state, so a multi-step objective
//! survives restart and replanning without losing its top-level goal.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub description: String,
    pub priority: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Succeeded,
    Failed { reason: String },
    Blocked { reason: String },
    Abandoned { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanTask {
    pub id: String,
    pub goal_id: String,
    pub description: String,
    pub deps: Vec<String>,
    pub status: TaskStatus,
    /// Task this one supersedes, if created by plan repair.
    pub supersedes: Option<String>,
    /// Machine-checkable evidence notes, newest last.
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Active,
    Paused,
    Cancelled,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetAmendment {
    pub previous_limit: u64,
    pub new_limit: u64,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanState {
    pub goals: Vec<Goal>,
    pub tasks: Vec<PlanTask>,
    pub lifecycle: Lifecycle,
    pub amendments: Vec<BudgetAmendment>,
}

impl Default for PlanState {
    fn default() -> Self {
        Self {
            goals: Vec::new(),
            tasks: Vec::new(),
            lifecycle: Lifecycle::Active,
            amendments: Vec::new(),
        }
    }
}

impl PlanState {
    pub fn add_goal(
        &mut self,
        id: impl Into<String>,
        description: impl Into<String>,
        priority: u32,
    ) -> Result<(), String> {
        let id = id.into();
        if self.goals.iter().any(|goal| goal.id == id) {
            return Err(format!("duplicate goal '{id}'"));
        }
        self.goals.push(Goal {
            id,
            description: description.into(),
            priority,
        });
        Ok(())
    }

    fn task_index(&self, id: &str) -> Option<usize> {
        self.tasks.iter().position(|task| task.id == id)
    }

    /// Register a task. Dependencies must already exist, and the resulting
    /// graph must stay acyclic; violations are rejected, never stored.
    pub fn add_task(
        &mut self,
        id: impl Into<String>,
        goal_id: &str,
        description: impl Into<String>,
        deps: Vec<String>,
    ) -> Result<(), String> {
        let id = id.into();
        if self.task_index(&id).is_some() {
            return Err(format!("duplicate task '{id}'"));
        }
        if !self.goals.iter().any(|goal| goal.id == goal_id) {
            return Err(format!("unknown goal '{goal_id}'"));
        }
        for dep in &deps {
            if dep == &id {
                return Err(format!("task '{id}' depends on itself"));
            }
            if self.task_index(dep).is_none() {
                return Err(format!("unknown dependency '{dep}'"));
            }
        }
        self.tasks.push(PlanTask {
            id,
            goal_id: goal_id.into(),
            description: description.into(),
            deps,
            status: TaskStatus::Pending,
            supersedes: None,
            evidence: Vec::new(),
        });
        if self.has_cycle() {
            self.tasks.pop();
            return Err("task dependencies contain a cycle".into());
        }
        Ok(())
    }

    fn has_cycle(&self) -> bool {
        fn visit(
            tasks: &[PlanTask],
            index: usize,
            temporary: &mut Vec<bool>,
            permanent: &mut Vec<bool>,
        ) -> bool {
            if permanent[index] {
                return false;
            }
            if temporary[index] {
                return true;
            }
            temporary[index] = true;
            for dep in &tasks[index].deps {
                if let Some(next) = tasks.iter().position(|task| &task.id == dep)
                    && visit(tasks, next, temporary, permanent)
                {
                    return true;
                }
            }
            temporary[index] = false;
            permanent[index] = true;
            false
        }
        let mut temporary = vec![false; self.tasks.len()];
        let mut permanent = vec![false; self.tasks.len()];
        (0..self.tasks.len()).any(|index| visit(&self.tasks, index, &mut temporary, &mut permanent))
    }

    fn task_mut(&mut self, id: &str) -> Result<&mut PlanTask, String> {
        let index = self
            .task_index(id)
            .ok_or_else(|| format!("unknown task '{id}'"))?;
        Ok(&mut self.tasks[index])
    }

    /// Tasks whose dependencies all succeeded and which are still pending.
    /// Ordered by goal priority first, then registration order, so the
    /// scheduler prefers important goals without starving declared order.
    /// Blocked, abandoned, and failed tasks never schedule; repair them
    /// explicitly with [`PlanState::repair`].
    pub fn ready_tasks(&self) -> Vec<String> {
        let mut ready: Vec<(u32, usize, String)> = self
            .tasks
            .iter()
            .enumerate()
            .filter(|(_, task)| {
                matches!(task.status, TaskStatus::Pending)
                    && task.deps.iter().all(|dep| {
                        self.tasks.iter().any(|other| {
                            other.id == *dep && matches!(other.status, TaskStatus::Succeeded)
                        })
                    })
            })
            .map(|(index, task)| {
                let priority = self
                    .goals
                    .iter()
                    .find(|goal| goal.id == task.goal_id)
                    .map(|goal| goal.priority)
                    .unwrap_or(u32::MAX);
                (priority, index, task.id.clone())
            })
            .collect();
        ready.sort();
        ready.into_iter().map(|(_, _, id)| id).collect()
    }

    pub fn mark_running(&mut self, id: &str) -> Result<(), String> {
        let task = self.task_mut(id)?;
        match task.status {
            TaskStatus::Pending => {
                task.status = TaskStatus::Running;
                Ok(())
            }
            _ => Err(format!("task '{id}' is not pending")),
        }
    }

    pub fn mark_succeeded(&mut self, id: &str, evidence: impl Into<String>) -> Result<(), String> {
        let task = self.task_mut(id)?;
        match task.status {
            TaskStatus::Pending | TaskStatus::Running => {
                task.evidence.push(evidence.into());
                task.status = TaskStatus::Succeeded;
                Ok(())
            }
            _ => Err(format!("task '{id}' cannot succeed from its current state")),
        }
    }

    pub fn mark_failed(&mut self, id: &str, reason: impl Into<String>) -> Result<(), String> {
        let task = self.task_mut(id)?;
        match task.status {
            TaskStatus::Pending | TaskStatus::Running => {
                task.status = TaskStatus::Failed {
                    reason: reason.into(),
                };
                Ok(())
            }
            _ => Err(format!("task '{id}' cannot fail from its current state")),
        }
    }

    pub fn mark_blocked(&mut self, id: &str, reason: impl Into<String>) -> Result<(), String> {
        let task = self.task_mut(id)?;
        match task.status {
            TaskStatus::Pending | TaskStatus::Running => {
                task.status = TaskStatus::Blocked {
                    reason: reason.into(),
                };
                Ok(())
            }
            _ => Err(format!("task '{id}' cannot block from its current state")),
        }
    }

    /// Evidence-driven plan repair: abandon a failed or blocked task with its
    /// reason preserved, and register a replacement that supersedes it. The
    /// top-level goal and every prior attempt stay in the record.
    pub fn repair(
        &mut self,
        failed_id: &str,
        replacement_id: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<(), String> {
        let replacement_id = replacement_id.into();
        let index = self
            .task_index(failed_id)
            .ok_or_else(|| format!("unknown task '{failed_id}'"))?;
        match &self.tasks[index].status {
            TaskStatus::Failed { .. } | TaskStatus::Blocked { .. } => {}
            _ => return Err(format!("task '{failed_id}' needs no repair")),
        }
        if self.task_index(&replacement_id).is_some() {
            return Err(format!("duplicate task '{replacement_id}'"));
        }
        let failed = self.tasks[index].clone();
        self.tasks[index].status = TaskStatus::Abandoned {
            reason: format!("superseded by '{replacement_id}'"),
        };
        // The replacement inherits dependencies so the repaired step waits
        // for the same prerequisites, not for the abandoned attempt.
        let result = self.add_task(
            replacement_id.clone(),
            &failed.goal_id,
            description,
            failed.deps.clone(),
        );
        if result.is_err() {
            self.tasks[index].status = failed.status;
            return result;
        }
        self.tasks
            .iter_mut()
            .find(|task| task.id == replacement_id)
            .expect("replacement was registered")
            .supersedes = Some(failed_id.into());
        Ok(())
    }

    pub fn goals_complete(&self) -> bool {
        !self.goals.is_empty()
            && self.tasks.iter().all(|task| {
                matches!(
                    task.status,
                    TaskStatus::Succeeded | TaskStatus::Abandoned { .. }
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> PlanState {
        let mut plan = PlanState::default();
        plan.add_goal("top", "repair the service", 1).unwrap();
        plan.add_task("a", "top", "first", vec![]).unwrap();
        plan.add_task("b", "top", "second", vec!["a".into()])
            .unwrap();
        plan.add_task("c", "top", "third", vec!["b".into()])
            .unwrap();
        plan
    }

    #[test]
    fn schedules_in_dependency_order() {
        let mut plan = example();
        assert_eq!(plan.ready_tasks(), vec!["a".to_string()]);
        plan.mark_running("a").unwrap();
        assert!(plan.ready_tasks().is_empty());
        plan.mark_succeeded("a", "a done").unwrap();
        assert_eq!(plan.ready_tasks(), vec!["b".to_string()]);
        plan.mark_failed("b", "b broke").unwrap();
        assert!(plan.ready_tasks().is_empty());
        assert!(!plan.goals_complete());
    }

    #[test]
    fn rejects_duplicates_unknown_deps_and_cycles() {
        let mut plan = PlanState::default();
        assert!(plan.add_task("a", "missing", "x", vec![]).is_err());
        plan.add_goal("top", "goal", 1).unwrap();
        assert!(plan.add_goal("top", "again", 1).is_err());
        plan.add_task("a", "top", "x", vec![]).unwrap();
        assert!(plan.add_task("a", "top", "y", vec![]).is_err());
        assert!(
            plan.add_task("b", "top", "y", vec!["ghost".into()])
                .is_err()
        );
        assert!(plan.add_task("c", "top", "y", vec!["c".into()]).is_err());
        // Dependencies must pre-exist, so add_task alone cannot express a
        // cycle today; the check guards deserialized or future edited data.
        // Construct one by hand to exercise the detector.
        plan.add_task("b", "top", "y", vec!["a".into()]).unwrap();
        plan.tasks.push(PlanTask {
            id: "c".into(),
            goal_id: "top".into(),
            description: "y".into(),
            deps: vec!["b".into()],
            status: TaskStatus::Pending,
            supersedes: None,
            evidence: Vec::new(),
        });
        plan.tasks
            .iter_mut()
            .find(|task| task.id == "a")
            .unwrap()
            .deps
            .push("c".into());
        assert!(plan.has_cycle());
    }

    #[test]
    fn ready_tasks_prefer_goal_priority_then_order() {
        let mut plan = PlanState::default();
        plan.add_goal("urgent", "u", 1).unwrap();
        plan.add_goal("later", "l", 9).unwrap();
        plan.add_task("slow", "later", "x", vec![]).unwrap();
        plan.add_task("also-slow", "later", "x", vec![]).unwrap();
        plan.add_task("fast", "urgent", "y", vec![]).unwrap();
        assert_eq!(
            plan.ready_tasks(),
            vec![
                "fast".to_string(),
                "slow".to_string(),
                "also-slow".to_string()
            ]
        );
    }

    #[test]
    fn repair_preserves_goal_and_failed_attempt() {
        let mut plan = example();
        plan.mark_succeeded("a", "ok").unwrap();
        plan.mark_failed("b", "wrong fix").unwrap();
        plan.repair("b", "b2", "corrected fix").unwrap();
        assert_eq!(plan.ready_tasks(), vec!["b2".to_string()]);
        let abandoned = plan.tasks.iter().find(|task| task.id == "b").unwrap();
        assert!(matches!(abandoned.status, TaskStatus::Abandoned { .. }));
        let replacement = plan.tasks.iter().find(|task| task.id == "b2").unwrap();
        assert_eq!(replacement.supersedes.as_deref(), Some("b"));
        assert_eq!(replacement.deps, vec!["a".to_string()]);
        assert_eq!(plan.goals.len(), 1);
        assert!(plan.repair("a", "a2", "unneeded").is_err());
    }
}
