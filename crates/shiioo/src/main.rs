//! `shiioo`: run agents on this machine.
//!
//! The run's event log lives in `<workspace>/.shiioo/shiioo.redb` and the
//! machine's call journal in `<workspace>/.shiioo/journal.jsonl`. Kill the
//! process at any point and `shiioo resume <run>` continues where the log
//! left off.

use clap::{Parser, Subcommand};
use serde::Deserialize;
use serde_json::{Value, json};
use shiioo_api::{ApiOptions, ApiState, Notifications};
use shiioo_core::{Event, EventStore, RunId, Seq, verify_chain};
use shiioo_rules::RuleSet;
use shiioo_runtime::{
    AgentSpec, AnthropicProvider, HarnessBilling, HarnessSpec, HookEndpoint, LocalMachine,
    ModelProvider, RunOutcome, RunSpec, RunStatus, Runtime, Toolbox, UnconfiguredProvider, Worker,
    anthropic::DEFAULT_MODEL, branch_for,
};
use shiioo_store::{Notifying, RedbStore};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

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
        /// Run the task with an external agent harness (`claude-code`)
        /// instead of Shiioo's own loop. Uses the harness's own login.
        #[arg(long, value_parser = ["claude-code"])]
        harness: Option<String>,
        /// Git repository the harness works on. The run gets its own
        /// worktree under .shiioo/worktrees and a `shiioo/<run>` branch.
        #[arg(long, requires = "harness")]
        repo: Option<PathBuf>,
        /// Model. Native runs default to claude-opus-5-5; harness runs to
        /// the harness's own default.
        #[arg(long)]
        model: Option<String>,
        /// Tools the agent may use. Native: Shiioo tool names (default
        /// read_file,write_file,shell). Harness: its allow rules (default
        /// "Read,Edit,Write,Glob,Grep,Bash(git *)").
        #[arg(long, value_delimiter = ',')]
        tools: Option<Vec<String>>,
        /// Extra instructions for the agent.
        #[arg(long, default_value = "")]
        system: String,
        #[arg(long, default_value_t = 50)]
        max_turns: u32,
        /// Reasoning effort for native runs: low, medium, high, xhigh or max.
        #[arg(long, default_value = "high")]
        effort: String,
        /// Harness permission mode (Claude Code: default, acceptEdits,
        /// plan, dontAsk, bypassPermissions).
        #[arg(long, default_value = "acceptEdits", requires = "harness")]
        permission_mode: String,
        /// Bill the harness to API credits (ANTHROPIC_API_KEY) instead of
        /// the subscription login.
        #[arg(long, requires = "harness")]
        api_billing: bool,
        /// Stop a harness run at this estimated spend (API billing only).
        #[arg(long, requires = "harness")]
        max_budget_usd: Option<f64>,
    },
    /// Continue a run, for example after the process was killed.
    Resume {
        run: String,
        #[arg(long, default_value = "high")]
        effort: String,
    },
    /// Print a run's events and verify its hash chain.
    Log { run: String },
    /// Called by an agent harness (not by people): answers one of its hook
    /// events by asking the Shiioo server. Reads the hook's JSON on stdin,
    /// prints the decision on stdout.
    #[command(hide = true)]
    Hook {
        #[command(subcommand)]
        event: HookEvent,
    },
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

#[derive(Subcommand)]
enum HookEvent {
    /// Claude Code `PreToolUse`. Environment: SHIIOO_SERVER_URL,
    /// SHIIOO_API_TOKEN, SHIIOO_RUN_ID (set by the worker on the harness).
    PreToolUse,
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
    if let Command::Hook { event } = &cli.command {
        return hook(event).await;
    }
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
            harness,
            repo,
            model,
            tools,
            system,
            max_turns,
            effort,
            permission_mode,
            api_billing,
            max_budget_usd,
        } => {
            let worker = worker(&store, &workspace, &data, &effort)?;
            let runtime = match harness.as_deref() {
                Some(_) => {
                    let mut h = HarnessSpec::claude_code();
                    h.repo = repo.as_ref().map(std::fs::canonicalize).transpose()?;
                    h.permission_mode = permission_mode;
                    h.max_budget_usd = max_budget_usd;
                    h.billing = if api_billing {
                        HarnessBilling::ApiKey
                    } else {
                        HarnessBilling::Subscription
                    };
                    Runtime::Harness(h)
                }
                None => Runtime::Native,
            };
            let is_harness = matches!(runtime, Runtime::Harness(_));
            let spec = RunSpec {
                task,
                agent: AgentSpec {
                    model: model.unwrap_or_else(|| {
                        if is_harness {
                            String::new()
                        } else {
                            DEFAULT_MODEL.into()
                        }
                    }),
                    system,
                    tools: tools.unwrap_or_else(|| {
                        if is_harness {
                            ["Read", "Edit", "Write", "Glob", "Grep", "Bash(git *)"]
                                .map(String::from)
                                .to_vec()
                        } else {
                            ["read_file", "write_file", "shell"]
                                .map(String::from)
                                .to_vec()
                        }
                    }),
                    max_turns,
                },
                runtime,
            };
            let run = worker.create(&spec).await?;
            println!("run {run}");
            if repo.is_some() {
                println!(
                    "branch {} in {}",
                    branch_for(run),
                    data.join("worktrees").join(run.to_string()).display()
                );
            }
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
        // Answered before the store was opened: hooks run in the harness's
        // working directory, not in an office.
        Command::Hook { .. } => Ok(ExitCode::SUCCESS),
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
    let config = Config::load(&data.join("config.toml"))?;
    let rules = load_rules(&data.join("rules.toml"))?;
    let store = Arc::new(Notifying::new(RedbStore::clone(store)));
    let provider = provider(effort);
    let machine = Arc::new(
        LocalMachine::open(workspace, data.join("journal.jsonl"), Toolbox::builtin())?
            .with_worktrees(data.join("worktrees")),
    );
    let token_path = data.join("api-token");
    let token = api_token(&token_path)?;
    let listener = tokio::net::TcpListener::bind(listen).await?;
    let addr = listener.local_addr()?;

    // Harness hooks call back into this server; a wildcard bind is reached
    // on loopback.
    let hook_host = if addr.ip().is_unspecified() {
        "127.0.0.1".to_owned()
    } else {
        addr.ip().to_string()
    };
    let hooks = HookEndpoint {
        server_url: format!("http://{hook_host}:{}", addr.port()),
        token: token.clone(),
        exe: std::env::current_exe()?,
    };
    let worker_store = Arc::clone(&store);
    let workers = Arc::new(move || {
        Worker::new(
            Arc::clone(&worker_store),
            Arc::clone(&provider),
            Arc::clone(&machine),
            format!("local-{}", std::process::id()).as_str(),
        )
        .with_hooks(hooks.clone())
    });

    let telegram = config.telegram_bot();
    let notifications: Option<Arc<dyn Notifications>> = telegram
        .as_ref()
        .map(|(bot, owner)| Arc::new(TelegramNotifications::new(Arc::clone(bot), *owner)) as _);
    let state = ApiState::with_options(
        store,
        workers,
        token,
        AgentSpec {
            model,
            system: String::new(),
            tools,
            max_turns: 50,
        },
        ApiOptions {
            rules,
            repos: config.repos.clone(),
            notifications,
        },
    );
    if let Some((bot, owner)) = telegram {
        let office = Arc::new(TelegramOffice {
            state: Arc::clone(&state),
        });
        tokio::spawn(shiioo_telegram::run_bot(bot, owner, office));
        println!("telegram bot: on, owner chat {owner}");
    } else {
        println!(
            "telegram bot: off (set SHIIOO_TELEGRAM_TOKEN and [telegram] owner_chat_id in {})",
            data.join("config.toml").display()
        );
    }
    println!(
        "listening on http://{addr}  (token: {}, {} repo(s), rules: {})",
        token_path.display(),
        config.repos.len(),
        data.join("rules.toml").display()
    );
    axum::serve(listener, shiioo_api::router(state))
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;
    Ok(ExitCode::SUCCESS)
}

/// `<data>/config.toml`: the office's settings that aren't secrets.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Config {
    telegram: TelegramConfig,
    /// Repository label → path. `/run <label> <task>` on the phone picks
    /// one; rules can be scoped to a label.
    repos: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TelegramConfig {
    /// Prefer the SHIIOO_TELEGRAM_TOKEN environment variable.
    token: Option<String>,
    /// The owner's Telegram user id. Only this account is obeyed.
    owner_chat_id: Option<i64>,
}

const DEFAULT_CONFIG: &str = r#"# Shiioo office configuration.

[telegram]
# The bot token comes from the SHIIOO_TELEGRAM_TOKEN environment variable
# (or `token = "..."` here). Create a bot with @BotFather.
# Your own Telegram user id (the bot obeys nobody else). @userinfobot tells you yours.
# owner_chat_id = 123456789

[repos]
# label = "/absolute/path/to/repository"
# shiioo = "/Users/me/code/shiioo"
"#;

impl Config {
    fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        if !path.exists() {
            std::fs::write(path, DEFAULT_CONFIG)?;
        }
        let text = std::fs::read_to_string(path)?;
        let config: Config =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        for (label, repo) in &config.repos {
            if !repo.join(".git").exists() {
                eprintln!(
                    "warning: repo {label} at {} is not a git repository",
                    repo.display()
                );
            }
        }
        Ok(config)
    }

    fn telegram_bot(&self) -> Option<(Arc<shiioo_telegram::Bot>, i64)> {
        let token = std::env::var("SHIIOO_TELEGRAM_TOKEN")
            .ok()
            .filter(|t| !t.is_empty())
            .or_else(|| self.telegram.token.clone())?;
        let owner = std::env::var("SHIIOO_TELEGRAM_OWNER")
            .ok()
            .and_then(|v| v.parse().ok())
            .or(self.telegram.owner_chat_id);
        match owner {
            Some(owner) => Some((Arc::new(shiioo_telegram::Bot::new(token)), owner)),
            None => {
                eprintln!(
                    "telegram: token set but no owner; set [telegram] owner_chat_id in config.toml \
                     (or SHIIOO_TELEGRAM_OWNER). The bot stays off."
                );
                None
            }
        }
    }
}

/// `<data>/rules.toml`, created with the defaults on first start.
fn load_rules(path: &Path) -> Result<RuleSet, Box<dyn std::error::Error>> {
    if !path.exists() {
        std::fs::write(path, RuleSet::defaults().to_toml())?;
    }
    let text = std::fs::read_to_string(path)?;
    Ok(RuleSet::from_toml(&text).map_err(|e| format!("{}: {e}", path.display()))?)
}

/// The phone's view of the office.
struct TelegramOffice {
    state: Arc<ApiState<RedbStore>>,
}

fn short(run: RunId) -> String {
    run.to_string()[..8].to_owned()
}

fn status_line(view: &shiioo_api::RunView) -> String {
    let (mark, detail) = match &view.status {
        RunStatus::Open => ("▶️", String::new()),
        RunStatus::WaitingApproval { summary, .. } => ("🔐", format!(" waiting: {summary}")),
        RunStatus::NeedsAttention { reason } => ("⚠️", format!(" {reason}")),
        RunStatus::Completed { .. } => ("✅", String::new()),
        RunStatus::Failed { reason } => ("❌", format!(" {reason}")),
        RunStatus::Cancelled { .. } => ("🛑", String::new()),
    };
    let task: String = view.task.chars().take(70).collect();
    format!("{mark} {} {task}{detail}", short(view.id))
}

impl shiioo_telegram::Office for TelegramOffice {
    fn repos(&self) -> Vec<String> {
        self.state.repos()
    }

    fn start_run(
        &self,
        repo: Option<String>,
        task: String,
    ) -> shiioo_telegram::BoxFuture<'_, Result<String, String>> {
        Box::pin(async move {
            let repos = self.state.repos();
            let label = match (repo, repos.as_slice()) {
                (Some(label), _) => Some(label),
                (None, [only]) => Some(only.clone()),
                (None, []) => None,
                (None, many) => {
                    return Err(format!(
                        "which repo? /run <repo> <task>, one of: {}",
                        many.join(", ")
                    ));
                }
            };
            let mut harness = HarnessSpec::claude_code();
            harness.repo = match &label {
                Some(l) => Some(
                    self.state
                        .repo_path(l)
                        .ok_or_else(|| format!("unknown repo {l}; /repos lists them"))?,
                ),
                None => None,
            };
            let spec = RunSpec {
                task: task.clone(),
                agent: AgentSpec {
                    model: String::new(),
                    system: String::new(),
                    tools: Vec::new(),
                    max_turns: 50,
                },
                runtime: Runtime::Harness(harness),
            };
            let run = self
                .state
                .start_run(spec)
                .await
                .map_err(|e| e.to_string())?;
            Ok(match label {
                Some(l) => format!(
                    "▶️ {} started on {l}: {task}\nbranch {}",
                    short(run),
                    branch_for(run)
                ),
                None => format!("▶️ {} started: {task}", short(run)),
            })
        })
    }

    fn status(&self) -> shiioo_telegram::BoxFuture<'_, String> {
        Box::pin(async move {
            match self.state.list_runs().await {
                Ok(runs) if runs.is_empty() => "No runs yet. Send me a task.".to_owned(),
                Ok(runs) => runs
                    .iter()
                    .take(15)
                    .map(status_line)
                    .collect::<Vec<_>>()
                    .join("\n"),
                Err(e) => format!("⚠️ {e}"),
            }
        })
    }

    fn log(&self, run: String) -> shiioo_telegram::BoxFuture<'_, String> {
        Box::pin(async move {
            let run = match self.state.resolve_run(&run).await {
                Ok(r) => r,
                Err(e) => return format!("⚠️ {e}"),
            };
            match self.state.log_text(run, 25).await {
                Ok(text) => text,
                Err(e) => format!("⚠️ {e}"),
            }
        })
    }

    fn cancel(&self, run: String) -> shiioo_telegram::BoxFuture<'_, Result<String, String>> {
        Box::pin(async move {
            let run = self
                .state
                .resolve_run(&run)
                .await
                .map_err(|e| e.to_string())?;
            self.state
                .cancel(run, "owner:telegram", "")
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!("🛑 {} will stop at its next step", short(run)))
        })
    }

    fn decide(
        &self,
        approval: String,
        allow: bool,
    ) -> shiioo_telegram::BoxFuture<'_, Result<String, String>> {
        Box::pin(async move {
            let run = self
                .state
                .decide(&approval, allow, "owner:telegram", None, false)
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "{} for {}",
                if allow { "allowed" } else { "denied" },
                short(run)
            ))
        })
    }
}

/// What the office tells the owner on its own initiative.
struct TelegramNotifications {
    notifier: shiioo_telegram::Notifier,
    /// approval id → (message id, text) of the question we sent, so the
    /// message can be marked when the request expires.
    asked: Mutex<HashMap<String, (i64, String)>>,
}

impl TelegramNotifications {
    fn new(bot: Arc<shiioo_telegram::Bot>, owner: i64) -> Self {
        Self {
            notifier: shiioo_telegram::Notifier::new(bot, owner),
            asked: Mutex::new(HashMap::new()),
        }
    }
}

impl Notifications for TelegramNotifications {
    fn approval_requested(
        &self,
        run: RunId,
        approval_id: String,
        summary: String,
    ) -> shiioo_api::BoxFuture<'_, ()> {
        Box::pin(async move {
            let text = format!("🔐 {} wants to run\n{summary}", short(run));
            match self.notifier.ask_approval(&approval_id, &text).await {
                Ok(message_id) => {
                    if let Ok(mut asked) = self.asked.lock() {
                        asked.insert(approval_id, (message_id, text));
                    }
                }
                Err(e) => eprintln!("telegram: could not ask for approval: {e}"),
            }
        })
    }

    fn approval_resolved(
        &self,
        approval_id: String,
        outcome: String,
    ) -> shiioo_api::BoxFuture<'_, ()> {
        Box::pin(async move {
            let asked = self
                .asked
                .lock()
                .ok()
                .and_then(|mut a| a.remove(&approval_id));
            if let Some((message_id, text)) = asked {
                self.notifier
                    .resolve_approval(message_id, &text, &outcome)
                    .await
                    .ok();
            }
        })
    }

    fn run_ended(
        &self,
        run: RunId,
        task: String,
        status: RunStatus,
    ) -> shiioo_api::BoxFuture<'_, ()> {
        Box::pin(async move {
            let task: String = task.chars().take(120).collect();
            let text = match status {
                RunStatus::Completed { output } => {
                    let output: String = output.chars().take(600).collect();
                    format!("✅ {} finished: {task}\n\n{output}", short(run))
                }
                RunStatus::Failed { reason } => {
                    format!("❌ {} failed: {task}\n{reason}", short(run))
                }
                RunStatus::Cancelled { reason } => format!("🛑 {} {reason}: {task}", short(run)),
                RunStatus::NeedsAttention { reason } => {
                    format!("⚠️ {} needs you: {task}\n{reason}", short(run))
                }
                RunStatus::Open | RunStatus::WaitingApproval { .. } => {
                    format!("⏸ {} stopped without finishing: {task}", short(run))
                }
            };
            self.notifier.notify(&text).await.ok();
        })
    }
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
    let provider = provider(effort);
    let machine = LocalMachine::open(workspace, data.join("journal.jsonl"), Toolbox::builtin())?
        .with_worktrees(data.join("worktrees"));
    Ok(Worker::new(
        Arc::clone(store),
        provider,
        Arc::new(machine),
        format!("local-{}", std::process::id()).as_str(),
    )
    .with_observer(Arc::new(print_event)))
}

/// Answers a harness hook by asking the server. Fails closed: if the
/// server can't be reached or doesn't answer, the tool call is denied with
/// the reason, and the harness tells the model.
async fn hook(event: &HookEvent) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let HookEvent::PreToolUse = event;
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
    let request: Value = serde_json::from_str(&input)?;
    let decision = match ask_server(&request).await {
        Ok(d) => d,
        Err(e) => json!({ "decision": "deny", "reason": format!("shiioo could not decide: {e}") }),
    };
    let allow = decision["decision"] == "allow";
    println!(
        "{}",
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": if allow { "allow" } else { "deny" },
                "permissionDecisionReason": decision["reason"].as_str().unwrap_or("decided by shiioo"),
            }
        })
    );
    Ok(ExitCode::SUCCESS)
}

async fn ask_server(request: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    let server = std::env::var("SHIIOO_SERVER_URL")?;
    let token = std::env::var("SHIIOO_API_TOKEN")?;
    let run = std::env::var("SHIIOO_RUN_ID")?;
    // The request may wait hours for the owner; the harness's own hook
    // timeout is the limit, not ours.
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(
            shiioo_runtime::harness::claude_code::HOOK_TIMEOUT_SECS + 60,
        ))
        .build()?;
    let response = http
        .post(format!("{server}/v1/runs/{run}/permissions"))
        .bearer_auth(token)
        .json(request)
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json().await?)
}

/// The native runtime's model provider. Without API credentials, native
/// runs fail with a clear message while harness runs still work.
fn provider(effort: &str) -> Arc<dyn ModelProvider> {
    match AnthropicProvider::from_env() {
        Ok(mut p) => {
            p.effort = effort.to_owned();
            Arc::new(p)
        }
        Err(_) => Arc::new(UnconfiguredProvider),
    }
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
            RunOutcome::Cancelled(reason) => {
                eprintln!("run {reason}");
                return Ok(ExitCode::from(4));
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
    println!("#{:<3} {}", ev.seq.0, shiioo_api::describe(ev));
}
