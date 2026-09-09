use harness_core::{AgentRuntime, FileEventStore, HeuristicModel, Objective, PermissionPolicy, RunOutcome, ToolRegistry};
use std::env;
use std::fs;
use std::path::PathBuf;

fn usage() -> ! {
    eprintln!("Usage: harness-cli --workspace <dir> --objective <text>");
    eprintln!("Example: harness-cli --workspace ./workspace --objective 'create file hello.txt with content hello agent'");
    std::process::exit(2);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mut workspace: Option<PathBuf> = None;
    let mut objective: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workspace" => workspace = args.next().map(PathBuf::from),
            "--objective" => objective = args.next(),
            "-h" | "--help" => usage(),
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    let workspace = workspace.unwrap_or_else(|| PathBuf::from("./workspace"));
    let objective = objective.unwrap_or_else(|| usage());
    fs::create_dir_all(&workspace)?;
    let events = FileEventStore::open(workspace.join(".harness/events.log"))?;
    let policy = PermissionPolicy::milestone_default(&workspace);
    let mut runtime = AgentRuntime::new(HeuristicModel, ToolRegistry::milestone_default(), policy, events);

    match runtime.run(Objective::new(objective))? {
        RunOutcome::Completed(summary) => {
            println!("COMPLETED: {summary}");
            Ok(())
        }
        RunOutcome::Failed(reason) => {
            eprintln!("FAILED: {reason}");
            std::process::exit(1);
        }
    }
}
