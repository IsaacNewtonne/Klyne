use crate::types::{Action, Objective, Observation, StepDecision};

pub trait Model: Send {
    fn name(&self) -> &str;
    fn decide(&mut self, objective: &Objective, history: &[(Action, Observation)]) -> StepDecision;
}

/// A dependency-free model used only for the first executable vertical slice.
/// It demonstrates the runtime contract with a narrow, deterministic objective grammar.
#[derive(Default)]
pub struct HeuristicModel;

impl HeuristicModel {
    pub(crate) fn parse_create(text: &str) -> Option<(String, String)> {
        // Accepted forms:
        //   create file hello.txt with content hello world
        //   write file hello.txt :: hello world
        let trimmed = text.trim();
        let lower = trimmed.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("create file ") {
            let marker = " with content ";
            let idx = rest.find(marker)?;
            let path_start = "create file ".len();
            let path_end = path_start + idx;
            let content_start = path_end + marker.len();
            return Some((
                trimmed[path_start..path_end].trim().to_string(),
                trimmed[content_start..].to_string(),
            ));
        }
        if lower.starts_with("write file ") {
            let sep = trimmed.find("::")?;
            let path = trimmed["write file ".len()..sep].trim().to_string();
            let contents = trimmed[sep + 2..].trim().to_string();
            return Some((path, contents));
        }
        None
    }
}

impl Model for HeuristicModel {
    fn name(&self) -> &str {
        "heuristic-milestone-model"
    }

    fn decide(&mut self, objective: &Objective, history: &[(Action, Observation)]) -> StepDecision {
        let Some((path, contents)) = Self::parse_create(&objective.text) else {
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
