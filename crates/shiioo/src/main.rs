//! `shiioo`: run agents on this machine.
//!
//! The run's event log lives in `<workspace>/.shiioo/shiioo.redb` and the
//! machine's call journal in `<workspace>/.shiioo/journal.jsonl`. Kill the
//! process at any point and `shiioo resume <run>` continues where the log
//! left off.

use clap::{Parser, Subcommand};
use serde_json::Value;
use shiioo_api::ApiState;
use shiioo_core::{Event, EventStore, Payload, RunId, Seq, verify_chain};
use shiioo_runtime::{
    AgentSpec, AnthropicProvider, LocalMachine, RunOutcome, RunSpec, Toolbox, Worker,
    anthropic::DEFAULT_MODEL,
};
use shiioo_store::{Notifying, RedbStore};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "shiioo", version, about = "Shiioo, the Virtual Company OS")]
struct Cli {
    /// Directory the agent works in. Shiioo keeps its data in `.shiioo/` inside it.
    #[arg(long, short, global = true, default_value = ".")]
    workspace: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start a run and work on it until it finishes.
    Run {
        /// What the agent should do.
        task: String,
        #[arg(long, default_value = DEFAULT_MODEL)]
        model: String,
        /// Tools the agent may use.
        #[arg(
            long,
            value_delimiter = ',',
            default_value = "read_file,write_file,shell"
        )]
        tools: Vec<String>,
        /// Extra instructions for the agent.
        #[arg(long, default_value = "")]
        system: String,
        #[arg(long, default_value_t = 50)]
        max_turns: u32,
        /// Reasoning effort: low, medium, high, xhigh or max.
        #[arg(long, default_value = "high")]
        effort: String,
    },
    /// Continue a run, for example after the process was killed.
    Resume {
        run: String,
        #[arg(long, default_value = "high")]
        effort: String,
    },
    /// Print a run's events and verify its hash chain.
    Log { run: String },
    /// Serve the HTTP API (runs over REST, live events over SSE) on this
    /// machine. Requests need the token stored in `.shiioo/api-token`.
    Serve {
        /// Address to listen on. Loopback by default: the API runs tools on
        /// this machine.
        #[arg(long, default_value = "127.0.0.1:7878")]
        listen: String,
        /// Model for runs that don't name an agent.
        #[arg(long, default_value = DEFAULT_MODEL)]
        model: String,
        #[arg(
            long,
            value_delimiter = ',',
            default_value = "read_file,write_file,shell"
        )]
        tools: Vec<String>,
        #[arg(long, default_value = "high")]
        effort: String,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let workspace = std::fs::canonicalize(&cli.workspace)?;
    let data = workspace.join(".shiioo");
    std::fs::create_dir_all(&data)?;
    let store = Arc::new(RedbStore::open(data.join("shiioo.redb"))?);

    match cli.command {
        Command::Log { run } => {
            let run = parse_run(&run)?;
            let log = store.read(run, Seq(0)).await?;
            log.iter().for_each(print_event);
            match verify_chain(&log) {
                Ok(()) => println!("chain verified: {} events", log.len()),
                Err(e) => println!("CHAIN BROKEN: {e}"),
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Run {
            task,
            model,
            tools,
            system,
            max_turns,
            effort,
        } => {
            let worker = worker(&store, &workspace, &data, &effort)?;
            let spec = RunSpec {
                task,
                agent: AgentSpec {
                    model,
                    system,
                    tools,
                    max_turns,
                },
            };
            let run = worker.create(&spec).await?;
            println!("run {run}");
            drive(&worker, run).await
        }
        Command::Resume { run, effort } => {
            let worker = worker(&store, &workspace, &data, &effort)?;
            drive(&worker, parse_run(&run)?).await
        }
        Command::Serve {
            listen,
            model,
            tools,
            effort,
        } => serve(&store, &workspace, &data, &listen, model, tools, &effort).await,
    }
}

async fn serve(
    store: &Arc<RedbStore>,
    workspace: &Path,
    data: &Path,
    listen: &str,
    model: String,
    tools: Vec<String>,
    effort: &str,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let store = Arc::new(Notifying::new(RedbStore::clone(store)));
    let mut provider = AnthropicProvider::from_env()?;
    provider.effort = effort.to_owned();
    let provider = Arc::new(provider);
    let machine = Arc::new(LocalMachine::open(
        workspace,
        data.join("journal.jsonl"),
        Toolbox::builtin(),
    )?);
    let worker_store = Arc::clone(&store);
    let workers = Arc::new(move || {
        Worker::new(
            Arc::clone(&worker_store),
            Arc::clone(&provider) as _,
            Arc::clone(&machine),
            format!("local-{}", std::process::id()).as_str(),
        )
    });
    let token_path = data.join("api-token");
    let token = api_token(&token_path)?;
    let state = ApiState::new(
        store,
        workers,
        token,
        AgentSpec {
            model,
            system: String::new(),
            tools,
            max_turns: 50,
        },
    );
    let listener = tokio::net::TcpListener::bind(listen).await?;
    println!(
        "listening on http://{}  (token: {})",
        listener.local_addr()?,
        token_path.display()
    );
    axum::serve(listener, shiioo_api::router(state))
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;
    Ok(ExitCode::SUCCESS)
}

/// Reads the API token, creating a random one (readable only by this user)
/// on first use.
fn api_token(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    if let Ok(token) = std::fs::read_to_string(path) {
        return Ok(token.trim().to_owned());
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    std::fs::write(path, &token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

fn worker(
    store: &Arc<RedbStore>,
    workspace: &Path,
    data: &Path,
    effort: &str,
) -> Result<Worker<RedbStore>, Box<dyn std::error::Error>> {
    let mut provider = AnthropicProvider::from_env()?;
    provider.effort = effort.to_owned();
    let machine = LocalMachine::open(workspace, data.join("journal.jsonl"), Toolbox::builtin())?;
    Ok(Worker::new(
        Arc::clone(store),
        Arc::new(provider),
        Arc::new(machine),
        format!("local-{}", std::process::id()).as_str(),
    )
    .with_observer(Arc::new(print_event)))
}

async fn drive(
    worker: &Worker<RedbStore>,
    run: RunId,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    // A lost lease on one machine means it expired (for example the laptop
    // slept); taking it again is safe, the fence protects the log.
    for _ in 0..3 {
        match worker.drive(run).await? {
            RunOutcome::Completed(text) => {
                println!("\n{text}");
                return Ok(ExitCode::SUCCESS);
            }
            RunOutcome::Failed(reason) => {
                eprintln!("run failed: {reason}");
                return Ok(ExitCode::from(1));
            }
            RunOutcome::NeedsAttention(reason) => {
                eprintln!("run needs attention: {reason}");
                return Ok(ExitCode::from(2));
            }
            RunOutcome::LostLease => eprintln!("lease lost, taking the run again"),
        }
    }
    Ok(ExitCode::from(3))
}

fn parse_run(s: &str) -> Result<RunId, uuid::Error> {
    Ok(RunId(uuid::Uuid::parse_str(s)?))
}

fn print_event(ev: &Event) {
    let line = match &*ev.payload {
        Payload::RunCreated { spec } => format!("run created: {}", one_line(&spec["task"], 100)),
        Payload::LeaseAcquired { worker, .. } => {
            format!("lease acquired by {} (epoch {})", worker.0, ev.epoch.0)
        }
        Payload::ModelResponseRecorded { model, response } => {
            let text: Vec<&str> = response["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect();
            format!("{model}: {}", one_line(&Value::from(text.join(" ")), 140))
        }
        Payload::ToolIntent { tool, input, .. } => format!("→ {tool} {}", one_line(input, 120)),
        Payload::ToolResult {
            output, is_error, ..
        } => format!(
            "{} {}",
            if *is_error { "✗" } else { "←" },
            one_line(output, 120)
        ),
        Payload::ToolOutcomeUnknown { call_id } => format!("? outcome of {} unknown", call_id.0),
        Payload::RunCompleted { .. } => "completed".into(),
        Payload::RunFailed { reason } => format!("failed: {reason}"),
    };
    println!("#{:<3} {line}", ev.seq.0);
}

fn one_line(v: &Value, max: usize) -> String {
    let s = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}
