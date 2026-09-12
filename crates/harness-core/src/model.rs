use crate::types::{Action, ModelUsage, Objective, Observation, StepDecision};

pub trait Model: Send {
    fn name(&self) -> &str;
    fn decide(&mut self, objective: &Objective, history: &[(Action, Observation)]) -> StepDecision;

    /// Cumulative spend so far. The default (no metering) keeps every
    /// existing model compiling; metered providers override it and the
    /// runtime accrues deltas into the persisted checkpoint.
    fn usage(&self) -> ModelUsage {
        ModelUsage::default()
    }
}

impl<M: Model + ?Sized> Model for Box<M> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn decide(&mut self, objective: &Objective, history: &[(Action, Observation)]) -> StepDecision {
        (**self).decide(objective, history)
    }

    fn usage(&self) -> ModelUsage {
        (**self).usage()
    }
}

/// A dependency-free model used only for the first executable vertical slice.
/// It demonstrates the runtime contract with a narrow, deterministic objective grammar.
#[derive(Default)]
pub struct HeuristicModel;

impl Model for HeuristicModel {
    fn name(&self) -> &str {
        "heuristic-milestone-model"
    }

    fn decide(&mut self, objective: &Objective, history: &[(Action, Observation)]) -> StepDecision {
        let Some(crate::verification::SuccessCriterion::FileContents {
            path,
            expected: contents,
        }) = crate::verification::SuccessCriterion::from_objective(&objective.text)
        else {
            return StepDecision::Fail(
                "milestone model supports: 'create file <relative-path> with content <text>' or 'write file <relative-path> :: <text>'".into(),
            );
        };

        match history {
            [] => StepDecision::Act(Action::WriteFile { path, contents }),
            [(Action::WriteFile { path: written, .. }, obs)] if obs.ok => {
                StepDecision::Verify(Action::ReadFile {
                    path: written.clone(),
                })
            }
            [(_, obs)] if !obs.ok => {
                StepDecision::Fail(format!("action failed: {}: {}", obs.summary, obs.data))
            }
            [
                (
                    Action::WriteFile {
                        contents: expected, ..
                    },
                    Observation { ok: true, .. },
                ),
                (Action::ReadFile { path, .. }, Observation { ok: true, data, .. }),
                ..,
            ] if data == expected => StepDecision::Complete(format!(
                "verified file '{path}' contains the requested content"
            )),
            [
                (
                    Action::WriteFile {
                        contents: expected, ..
                    },
                    Observation { ok: true, .. },
                ),
                (Action::ReadFile { path, .. }, Observation { ok: true, data, .. }),
                ..,
            ] => StepDecision::Fail(format!(
                "verification mismatch for '{path}': expected {} bytes, observed {} bytes",
                expected.len(),
                data.len()
            )),
            [_, (_, obs)] if !obs.ok => StepDecision::Fail(format!(
                "verification action failed: {}: {}",
                obs.summary, obs.data
            )),
            _ => StepDecision::Fail("unexpected runtime history shape".into()),
        }
    }
}
