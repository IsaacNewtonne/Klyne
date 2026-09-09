use harness_core::{
    AgentRuntime, HeuristicModel, Objective, PermissionPolicy, RunOutcome, SqliteEventStore,
    ToolRegistry,
};
use std::env;
use std::fs;
use std::path::PathBuf;

fn usage() -> ! {
    eprintln!(
        "Usage: harness-cli --workspace <dir> [--objective <text> | --resume | --reconcile | --events] [--database <path>]"
    );
    eprintln!(
        "Example: harness-cli --workspace ./workspace --objective 'create file hello.txt with content hello agent'"
    );
    std::process::exit(2);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mut workspace: Option<PathBuf> = None;
    let mut objective: Option<String> = None;
    let mut database: Option<PathBuf> = None;
    let mut resume = false;
    let mut reconcile = false;
    let mut show_events = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workspace" => workspace = args.next().map(PathBuf::from),
            "--objective" => objective = args.next(),
            "--database" => {
                database = Some(PathBuf::from(
                    args.next().ok_or("--database requires a path")?,
                ))
            }
            "--resume" => resume = true,
            "--reconcile" => reconcile = true,
            "--events" => show_events = true,
            "-h" | "--help" => usage(),
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    let workspace = workspace.unwrap_or_else(|| PathBuf::from("./workspace"));
    if usize::from(objective.is_some())
        + usize::from(resume)
        + usize::from(reconcile)
        + usize::from(show_events)
        != 1
    {
        usage();
    }
    fs::create_dir_all(&workspace)?;
    let events =
        SqliteEventStore::open(database.unwrap_or_else(|| workspace.join(".harness/run.sqlite3")))?;
    if show_events {
        for event in events.events()? {
            println!("{}\t{}\t{:?}", event.seq, event.kind, event.detail);
        }
        return Ok(());
    }
    let policy = PermissionPolicy::milestone_default(&workspace);
    let mut runtime = AgentRuntime::new(
        HeuristicModel,
        ToolRegistry::milestone_default(),
        policy,
        events,
    );

    let outcome = if reconcile {
        runtime.reconcile_and_resume()?
    } else if resume {
        runtime.resume()?
    } else {
        runtime.run(Objective::new(objective.ok_or("missing objective")?))?
    };
    match outcome {
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
