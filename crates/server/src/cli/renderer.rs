use crossterm::style::{Attribute, Color, SetAttribute, SetForegroundColor, ResetColor};
use shiioo_core::types::{ContentBlockType, DeltaContent, LlmContentBlock, LlmError, StreamEvent};
use std::io::{self, Write};
use tokio_stream::StreamExt;

/// Collected result from streaming an assistant response.
pub struct StreamedResponse {
    pub content_blocks: Vec<LlmContentBlock>,
    pub stop_reason: Option<String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Print the welcome banner when the REPL starts.
pub fn print_banner() {
    let version = env!("CARGO_PKG_VERSION");
    println!();
    print_colored("Shiioo", Color::Magenta, true);
    println!(" v{version}");
    print_colored("Virtual Company OS", Color::DarkGrey, false);
    println!();
    println!();
    print_colored("Your Chief of Staff is ready.", Color::Cyan, false);
    println!(" Type a message to begin, or /help for commands.");
    println!();
}

/// Stream an assistant response, accumulating content blocks (text + tool_use).
///
/// When `show_header` is true, prints the "Chief of Staff" header.
/// Returns a `StreamedResponse` with all content blocks and usage info.
pub async fn stream_response(
    mut stream: std::pin::Pin<
        Box<dyn futures_core::Stream<Item = Result<StreamEvent, LlmError>> + Send>,
    >,
    show_header: bool,
) -> Result<StreamedResponse, LlmError> {
    if show_header {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        let _ = write!(out, "{}", SetForegroundColor(Color::Green));
        let _ = write!(out, "{}", SetAttribute(Attribute::Bold));
        let _ = write!(out, "Chief of Staff");
        let _ = write!(out, "{}", SetAttribute(Attribute::Reset));
        let _ = writeln!(out, "{}", ResetColor);
        let _ = out.flush();
    }

    // Per-index accumulators
    struct BlockAccum {
        block_type: ContentBlockType,
        text: String,
        json_parts: String,
    }

    let mut accums: Vec<Option<BlockAccum>> = Vec::new();
    let mut finished_blocks: Vec<(u32, LlmContentBlock)> = Vec::new();
    let mut input_tokens = 0u32;
    let mut output_tokens = 0u32;
    let mut stop_reason: Option<String> = None;

    while let Some(event) = stream.next().await {
        match event {
            Ok(StreamEvent::ContentBlockStart { index, block_type }) => {
                let idx = index as usize;
                while accums.len() <= idx {
                    accums.push(None);
                }
                accums[idx] = Some(BlockAccum {
                    block_type,
                    text: String::new(),
                    json_parts: String::new(),
                });
            }
            Ok(StreamEvent::ContentBlockDelta { index, delta }) => {
                let idx = index as usize;
                if let Some(Some(accum)) = accums.get_mut(idx) {
                    match &delta {
                        DeltaContent::Text(text) => {
                            accum.text.push_str(text);
                            let stdout = io::stdout();
                            let mut out = stdout.lock();
                            let _ = write!(out, "{text}");
                            let _ = out.flush();
                        }
                        DeltaContent::InputJson(partial) => {
                            accum.json_parts.push_str(partial);
                        }
                    }
                }
            }
            Ok(StreamEvent::ContentBlockStop { index }) => {
                let idx = index as usize;
                if let Some(accum) = accums.get_mut(idx).and_then(|a| a.take()) {
                    let block = match accum.block_type {
                        ContentBlockType::Text => LlmContentBlock::Text { text: accum.text },
                        ContentBlockType::ToolUse { id, name } => {
                            let input = serde_json::from_str(&accum.json_parts)
                                .unwrap_or(serde_json::Value::Object(Default::default()));
                            LlmContentBlock::ToolUse { id, name, input }
                        }
                    };
                    finished_blocks.push((index, block));
                }
            }
            Ok(StreamEvent::MessageDelta {
                stop_reason: sr,
            }) => {
                stop_reason = sr;
            }
            Ok(StreamEvent::MessageStop) => {
                break;
            }
            Ok(StreamEvent::Usage {
                input_tokens: inp,
                output_tokens: out_tok,
            }) => {
                input_tokens = inp;
                output_tokens = out_tok;
            }
            Ok(_) => {}
            Err(e) => {
                let stdout = io::stdout();
                let mut out = stdout.lock();
                let _ = writeln!(out);
                let _ = write!(out, "{}", SetForegroundColor(Color::Red));
                let _ = writeln!(out, "Stream error: {e:?}");
                let _ = write!(out, "{}", ResetColor);
                return Err(e);
            }
        }
    }

    // Final newlines after text output
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(out);
    let _ = writeln!(out);
    drop(out);

    // Sort blocks by index and collect
    finished_blocks.sort_by_key(|(idx, _)| *idx);
    let content_blocks = finished_blocks.into_iter().map(|(_, b)| b).collect();

    Ok(StreamedResponse {
        content_blocks,
        stop_reason,
        input_tokens,
        output_tokens,
    })
}

/// Print a tool call being made (for user visibility).
pub fn print_tool_call(name: &str, input: &serde_json::Value) {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let _ = write!(out, "{}", SetForegroundColor(Color::Yellow));
    let _ = write!(out, "  [calling {name}]");
    let _ = write!(out, "{}", ResetColor);

    // Show a compact summary of the input params
    if let Some(obj) = input.as_object() {
        let params: Vec<String> = obj
            .iter()
            .map(|(k, v)| {
                let val = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("{k}={val}")
            })
            .collect();
        if !params.is_empty() {
            let _ = write!(out, " {}", params.join(", "));
        }
    }
    let _ = writeln!(out);
    let _ = out.flush();
}

/// Print a tool result (for user visibility).
pub fn print_tool_result(name: &str, result: &str, is_error: bool) {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let color = if is_error { Color::Red } else { Color::DarkGrey };
    let _ = write!(out, "{}", SetForegroundColor(color));

    // Show first line or up to 120 chars
    let preview = result.lines().next().unwrap_or(result);
    let preview = if preview.len() > 120 {
        &preview[..preview.floor_char_boundary(120)]
    } else {
        preview
    };

    let label = if is_error { "error" } else { "done" };
    let _ = writeln!(out, "  [{name} {label}] {preview}");
    let _ = write!(out, "{}", ResetColor);
    let _ = out.flush();
}

/// Print a system/info message.
pub fn print_info(text: &str) {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let _ = write!(out, "{}", SetForegroundColor(Color::DarkGrey));
    let _ = writeln!(out, "{text}");
    let _ = write!(out, "{}", ResetColor);
}

/// Print an error message.
pub fn print_error(text: &str) {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let _ = write!(out, "{}", SetForegroundColor(Color::Red));
    let _ = writeln!(out, "{text}");
    let _ = write!(out, "{}", ResetColor);
}

/// Print a table of key-value pairs.
pub fn print_table(rows: &[(&str, String)]) {
    let max_key = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for (key, value) in rows {
        let _ = write!(out, "{}", SetForegroundColor(Color::Cyan));
        let _ = write!(out, "  {key:>width$}", width = max_key);
        let _ = write!(out, "{}", ResetColor);
        let _ = writeln!(out, "  {value}");
    }
    let _ = writeln!(out);
}

/// Print a colored string inline.
fn print_colored(text: &str, color: Color, bold: bool) {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let _ = write!(out, "{}", SetForegroundColor(color));
    if bold {
        let _ = write!(out, "{}", SetAttribute(Attribute::Bold));
    }
    let _ = write!(out, "{text}");
    if bold {
        let _ = write!(out, "{}", SetAttribute(Attribute::Reset));
    }
    let _ = write!(out, "{}", ResetColor);
}

/// Print the help text for slash commands.
pub fn print_help() {
    println!();
    print_colored("Available commands:", Color::Cyan, true);
    println!();
    println!();
    let commands = [
        ("/help", "Show this help message"),
        ("/quit, /exit", "Exit the REPL"),
        ("/clear", "Clear conversation history"),
        ("/status", "Show company overview"),
        ("/employees", "List all employees"),
        ("/cost", "Show session token/cost usage"),
        ("/model <name>", "Switch the LLM model"),
    ];

    let stdout = io::stdout();
    let mut out = stdout.lock();
    for (cmd, desc) in &commands {
        let _ = write!(out, "{}", SetForegroundColor(Color::Yellow));
        let _ = write!(out, "  {cmd:<20}");
        let _ = write!(out, "{}", ResetColor);
        let _ = writeln!(out, "{desc}");
    }
    let _ = writeln!(out);
}
