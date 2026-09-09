use harness_core::event_store::EventStore;
use harness_core::{
    AgentRuntime, HeuristicModel, Model, Objective, PermissionPolicy, RunOutcome, SqliteEventStore,
    ToolRegistry,
};
use harness_provider::{OpenAiCompat, ProviderConfig};
use std::env;
use std::fs;
use std::path::PathBuf;

fn usage() -> ! {
    eprintln!(
        "Usage: harness-cli --workspace <dir> [--objective <text> | --resume | --reconcile | --events | --tools | --inspect] [--database <path>] [--max-tool-calls <integer> (new runs only)] [--openai-compat]"
    );
    eprintln!(
        "Example: harness-cli --workspace ./workspace --objective 'create file hello.txt with content hello agent'"
    );
    eprintln!(
        "With --openai-compat, the model is an OpenAI-compatible endpoint from HARNESS_MODEL_ENDPOINT and HARNESS_MODEL_NAME, with an optional key in HARNESS_MODEL_API_KEY."
    );
    std::process::exit(2);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mut workspace: Option<PathBuf> = None;
    let mut objective: Option<String> = None;
    let mut database: Option<PathBuf> = None;
    let mut resume = false;
    let mut max_tool_calls: Option<u64> = None;
    let mut reconcile = false;
    let mut show_events = false;
    let mut show_tools = false;
    let mut show_inspect = false;
    let mut openai_compat = false;
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
            "--max-tool-calls" => {
                max_tool_calls = Some(
                    args.next()
                        .ok_or("--max-tool-calls requires an integer")?
                        .parse()?,
                )
            }
            "--reconcile" => reconcile = true,
            "--events" => show_events = true,
            "--tools" => show_tools = true,
            "--inspect" => show_inspect = true,
            "--openai-compat" => openai_compat = true,
            "-h" | "--help" => usage(),
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    let workspace = workspace.unwrap_or_else(|| PathBuf::from("./workspace"));
    if max_tool_calls.is_some() && objective.is_none() {
        return Err("--max-tool-calls only applies to a new objective".into());
    }
    if usize::from(objective.is_some())
        + usize::from(resume)
        + usize::from(reconcile)
        + usize::from(show_events)
        + usize::from(show_tools)
        + usize::from(show_inspect)
        != 1
    {
        usage();
    }
    fs::create_dir_all(&workspace)?;
    let events =
        SqliteEventStore::open(database.unwrap_or_else(|| workspace.join(".harness/run.sqlite3")))?;
    if show_tools {
        let registry = ToolRegistry::milestone_default();
        println!(
            "{}",
            serde_json::to_string_pretty(&registry.descriptors()).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if show_events {
        for event in events.events()? {
            println!("{}\t{}\t{:?}", event.seq, event.kind, event.detail);
        }
        return Ok(());
    }
    if show_inspect {
        let mut events = events;
        println!(
            "{}",
            serde_json::to_string_pretty(&harness_core::inspect::inspect_run(&mut events)?)
                .map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    let model: Box<dyn Model> = if openai_compat {
        let mut config = ProviderConfig::new(
            env::var("HARNESS_MODEL_ENDPOINT")
                .map_err(|_| "--openai-compat requires HARNESS_MODEL_ENDPOINT")?,
            env::var("HARNESS_MODEL_NAME")
                .map_err(|_| "--openai-compat requires HARNESS_MODEL_NAME")?,
        );
        if env::var("HARNESS_MODEL_API_KEY").is_ok() {
            config = config.with_api_key_env("HARNESS_MODEL_API_KEY");
        }
        Box::new(OpenAiCompat::new(config).map_err(|e| e.to_string())?)
    } else {
        Box::new(HeuristicModel)
    };
    let policy = PermissionPolicy::milestone_default(&workspace);
    let mut runtime = AgentRuntime::new(model, ToolRegistry::milestone_default(), policy, events);
    if let Some(limit) = max_tool_calls {
        runtime = runtime.with_max_tool_calls(limit);
    }

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
