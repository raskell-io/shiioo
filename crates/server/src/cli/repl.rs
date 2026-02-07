use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use shiioo_core::storage::AgentStore;
use shiioo_core::types::LlmContentBlock;

use crate::config::{AppState, ServerConfig};

use super::conversation::Conversation;
use super::renderer;
use super::tools;

/// Run the conversational REPL.
pub async fn run(config: ServerConfig) -> Result<()> {
    let state = AppState::new(&config)?;

    renderer::print_banner();

    // Show capacity source status
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        renderer::print_info("Using Claude (anthropic) from environment.");
    } else {
        renderer::print_info(
            "No ANTHROPIC_API_KEY set. Using mock responses. \
             Set the env var and restart for real conversations.",
        );
    }
    println!();

    let mut conversation = Conversation::new(state).await?;
    let mut rl = DefaultEditor::new()?;

    // Load history if it exists
    let history_path = dirs_home().join(".shiioo_history");
    let _ = rl.load_history(&history_path);

    loop {
        let prompt = format!("\x1b[1;34mCEO >\x1b[0m ");
        match rl.readline(&prompt) {
            Ok(line) => {
                let input = line.trim();
                if input.is_empty() {
                    continue;
                }

                let _ = rl.add_history_entry(input);

                // Handle slash commands
                if input.starts_with('/') {
                    match handle_slash_command(input, &mut conversation).await {
                        SlashResult::Continue => continue,
                        SlashResult::Quit => break,
                    }
                }

                // Send to conversation with tool-use loop
                if let Err(e) = send_with_tools(&mut conversation, input).await {
                    renderer::print_error(&format!("Error: {e}"));
                }
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl-C: cancel current input, don't quit
                println!();
                continue;
            }
            Err(ReadlineError::Eof) => {
                // Ctrl-D: quit
                break;
            }
            Err(e) => {
                renderer::print_error(&format!("Input error: {e}"));
                break;
            }
        }
    }

    // Save history
    let _ = rl.save_history(&history_path);

    renderer::print_info("Goodbye, CEO.");
    Ok(())
}

const MAX_TOOL_ROUNDS: usize = 10;

/// Send a message with tool-use support, running the agentic loop.
async fn send_with_tools(conversation: &mut Conversation, input: &str) -> Result<()> {
    let tool_defs = tools::chief_of_staff_tools();
    let mut rounds = 0usize;
    let mut user_input: Option<&str> = Some(input);

    loop {
        let stream = conversation
            .send_streaming_with_tools(user_input, &tool_defs)
            .await?;

        let response = renderer::stream_response(stream, rounds == 0)
            .await
            .map_err(|e| anyhow::anyhow!("Stream error: {e:?}"))?;

        conversation.finish_streaming_response(&response);

        if response.stop_reason.as_deref() == Some("tool_use") && rounds < MAX_TOOL_ROUNDS {
            // Execute all tool_use blocks
            let mut results: Vec<(String, String, bool)> = Vec::new();

            for block in &response.content_blocks {
                if let LlmContentBlock::ToolUse {
                    id,
                    name,
                    input: tool_input,
                } = block
                {
                    renderer::print_tool_call(name, tool_input);
                    let exec_result =
                        tools::execute_tool(name, tool_input, conversation.state()).await;
                    renderer::print_tool_result(name, &exec_result.content, exec_result.is_error);
                    results.push((
                        id.clone(),
                        exec_result.content,
                        exec_result.is_error,
                    ));
                }
            }

            conversation.add_tool_results(results);
            user_input = None; // Continuation round — no new user text
            rounds += 1;
        } else {
            break;
        }
    }

    Ok(())
}

enum SlashResult {
    Continue,
    Quit,
}

async fn handle_slash_command(input: &str, conversation: &mut Conversation) -> SlashResult {
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    let command = parts[0];
    let arg = parts.get(1).map(|s| s.trim());

    match command {
        "/quit" | "/exit" => return SlashResult::Quit,

        "/help" => {
            renderer::print_help();
        }

        "/clear" => {
            conversation.clear();
            renderer::print_info("Conversation cleared.");
        }

        "/status" => {
            show_status(conversation).await;
        }

        "/employees" => {
            show_employees(conversation).await;
        }

        "/cost" => {
            show_cost(conversation);
        }

        "/model" => {
            if let Some(model) = arg {
                conversation.set_model(model);
                renderer::print_info(&format!("Switched to model: {model}"));
            } else {
                renderer::print_info(&format!("Current model: {}", conversation.model()));
                renderer::print_info("Usage: /model <model-name>");
            }
        }

        _ => {
            renderer::print_error(&format!("Unknown command: {command}. Type /help for available commands."));
        }
    }

    SlashResult::Continue
}

async fn show_status(conversation: &Conversation) {
    let state = conversation.state();
    let agents = state.agent_store.list_agents().await.unwrap_or_default();

    let active = agents.iter().filter(|a| a.status == shiioo_core::agent::AgentStatus::Active).count();
    let total = agents.len();

    let sources = state.capacity_broker.list_sources();

    let (input_tokens, output_tokens, cost) = conversation.session_usage();

    renderer::print_table(&[
        ("Employees", format!("{active} active / {total} total")),
        ("LLM Sources", format!("{}", sources.len())),
        ("Model", conversation.model().to_string()),
        ("Session tokens", format!("{} in / {} out", input_tokens, output_tokens)),
        ("Session cost", format!("${:.4}", cost)),
    ]);
}

async fn show_employees(conversation: &Conversation) {
    let state = conversation.state();
    let agents = state.agent_store.list_agents().await.unwrap_or_default();

    if agents.is_empty() {
        renderer::print_info("No employees hired yet.");
        return;
    }

    println!();
    let rows: Vec<(&str, String)> = agents
        .iter()
        .map(|a| {
            let name: &str = &a.name;
            let info = format!("{} [{}]", a.description, a.status);
            (name, info)
        })
        .collect();

    // Can't use print_table directly because of borrowing, so print manually
    let max_name = rows.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
    for (name, info) in &rows {
        println!("  \x1b[36m{name:>width$}\x1b[0m  {info}", width = max_name);
    }
    println!();
}

fn show_cost(conversation: &Conversation) {
    let (input_tokens, output_tokens, cost) = conversation.session_usage();

    renderer::print_table(&[
        ("Input tokens", format!("{input_tokens}")),
        ("Output tokens", format!("{output_tokens}")),
        ("Total tokens", format!("{}", input_tokens + output_tokens)),
        ("Session cost", format!("${cost:.6}")),
    ]);
}

/// Get the user's home directory.
fn dirs_home() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}
