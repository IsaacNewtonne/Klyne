//! Serial DAG scheduler: dependencies govern readiness; Done is worker-reported.
use crate::{chat::Task, err};
use std::io;

pub fn validate(tasks: &[Task]) -> io::Result<()> {
    if tasks.is_empty() || tasks.len() > 6 {
        return Err(err("Execution graph requires 1-6 steps"));
    }
    for (index, task) in tasks.iter().enumerate() {
        let mut seen = std::collections::HashSet::new();
        if task
            .depends_on
            .iter()
            .any(|id| *id == 0 || *id > tasks.len() || *id == index + 1 || !seen.insert(*id))
        {
            return Err(err("Invalid or duplicate execution dependency"));
        }
    }
    let mut visited = vec![false; tasks.len()];
    for _ in 0..tasks.len() {
        let Some(index) = tasks
            .iter()
            .enumerate()
            .position(|(i, task)| !visited[i] && task.depends_on.iter().all(|id| visited[id - 1]))
        else {
            return Err(err("Execution graph contains a cycle"));
        };
        visited[index] = true;
    }
    Ok(())
}

pub fn next(tasks: &[Task]) -> io::Result<Option<usize>> {
    validate(tasks)?;
    if tasks.iter().all(|t| t.status == "Done") {
        return Ok(None);
    }
    tasks
        .iter()
        .position(|task| {
            !matches!(
                task.status.as_str(),
                "Done" | "Awaiting approval" | "Declined"
            ) && task
                .depends_on
                .iter()
                .all(|id| tasks[id - 1].status == "Done")
        })
        .map_or(Ok(None), |index| Ok(Some(index)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task(deps: &[usize]) -> Task {
        serde_json::from_value(serde_json::json!({"agent":"worker","instruction":"work","status":"Queued","depends_on":deps})).unwrap()
    }
    #[test]
    fn dependencies_override_array_order_and_survive_restart() {
        let mut tasks = vec![task(&[2]), task(&[]), task(&[1, 2])];
        assert_eq!(next(&tasks).unwrap(), Some(1));
        tasks[1].status = "Done".into();
        let saved = serde_json::to_string(&tasks).unwrap();
        let mut loaded: Vec<Task> = serde_json::from_str(&saved).unwrap();
        assert_eq!(next(&loaded).unwrap(), Some(0));
        loaded[0].status = "Working".into();
        assert_eq!(next(&loaded).unwrap(), Some(0));
        loaded[0].status = "Done".into();
        assert_eq!(next(&loaded).unwrap(), Some(2));
        loaded[2].status = "Done".into();
        assert_eq!(next(&loaded).unwrap(), None);
    }
    #[test]
    fn malformed_graphs_fail_before_execution() {
        for graph in [
            vec![task(&[2]), task(&[1])],
            vec![task(&[1])],
            vec![task(&[0])],
            vec![task(&[2])],
            vec![task(&[]), task(&[1, 1])],
        ] {
            assert!(validate(&graph).is_err());
        }
    }
}
