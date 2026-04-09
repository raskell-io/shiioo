use std::collections::VecDeque;

use shiioo_core::agent::{Agent, AgentId, AgentStatus, AgentTeam};
use shiioo_core::analytics::ExecutionTrace;
use shiioo_core::storage::AgentStore;
use shiioo_core::types::ApprovalStatus;

use crate::cli::tools::execute_tool;
use crate::config::AppState;

/// Which screen is currently displayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Dashboard,
    EmployeeDetail(AgentId),
    Logs,
    Teams,
}

/// Cached data from the service layer, refreshed on each tick.
#[derive(Debug, Default)]
pub struct DashboardData {
    pub employees: Vec<Agent>,
    pub active_count: usize,
    pub paused_count: usize,
    pub suspended_count: usize,
    pub team_count: usize,
    pub pending_approvals: usize,
    pub capacity_sources: usize,
    pub cost_24h: f64,
    pub recent_traces: Vec<ExecutionTrace>,
    pub teams: Vec<AgentTeam>,
}

/// A message displayed in the command result area.
#[derive(Debug, Clone)]
pub struct CommandMessage {
    pub text: String,
    pub is_error: bool,
}

/// Main TUI application state.
pub struct App {
    pub state: AppState,
    pub current_view: View,
    pub data: DashboardData,
    pub selected: usize,
    pub log_selected: usize,
    pub should_quit: bool,
    // Command bar
    pub command_mode: bool,
    pub command_input: String,
    pub command_cursor: usize,
    pub command_messages: VecDeque<CommandMessage>,
}

impl App {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            current_view: View::Dashboard,
            data: DashboardData::default(),
            selected: 0,
            log_selected: 0,
            should_quit: false,
            command_mode: false,
            command_input: String::new(),
            command_cursor: 0,
            command_messages: VecDeque::new(),
        }
    }

    /// Refresh cached data from the service layer.
    pub async fn refresh(&mut self) {
        // Employees
        if let Ok(employees) = self.state.agent_store.list_agents().await {
            self.data.active_count = employees
                .iter()
                .filter(|a| a.status == AgentStatus::Active)
                .count();
            self.data.paused_count = employees
                .iter()
                .filter(|a| a.status == AgentStatus::Paused)
                .count();
            self.data.suspended_count = employees
                .iter()
                .filter(|a| a.status == AgentStatus::Suspended)
                .count();
            self.data.employees = employees;
        }

        // Teams
        let teams = self.state.agent_orchestrator.list_teams().await;
        self.data.team_count = teams.len();
        self.data.teams = teams;

        // Approvals
        let approvals = self.state.approval_manager.list_approvals();
        self.data.pending_approvals = approvals
            .iter()
            .filter(|a| a.status == ApprovalStatus::Pending)
            .count();

        // Capacity
        self.data.capacity_sources = self.state.capacity_broker.list_sources().len();
        let since = chrono::Utc::now() - chrono::Duration::days(1);
        self.data.cost_24h = self.state.capacity_broker.get_total_cost(since);

        // Recent activity
        self.data.recent_traces = self.state.analytics.get_recent_traces(50);

        // Clamp selections
        if !self.data.employees.is_empty() && self.selected >= self.data.employees.len() {
            self.selected = self.data.employees.len() - 1;
        }
        if !self.data.recent_traces.is_empty() && self.log_selected >= self.data.recent_traces.len()
        {
            self.log_selected = self.data.recent_traces.len() - 1;
        }
    }

    pub fn select_next(&mut self) {
        if !self.data.employees.is_empty() {
            self.selected = (self.selected + 1) % self.data.employees.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.data.employees.is_empty() {
            if self.selected == 0 {
                self.selected = self.data.employees.len() - 1;
            } else {
                self.selected -= 1;
            }
        }
    }

    pub fn log_select_next(&mut self) {
        if !self.data.recent_traces.is_empty() {
            self.log_selected = (self.log_selected + 1) % self.data.recent_traces.len();
        }
    }

    pub fn log_select_prev(&mut self) {
        if !self.data.recent_traces.is_empty() {
            if self.log_selected == 0 {
                self.log_selected = self.data.recent_traces.len() - 1;
            } else {
                self.log_selected -= 1;
            }
        }
    }

    /// Open the detail view for the currently selected employee.
    pub fn open_employee_detail(&mut self) {
        if let Some(agent) = self.data.employees.get(self.selected) {
            self.current_view = View::EmployeeDetail(agent.id.clone());
        }
    }

    /// Go back to the dashboard.
    pub fn go_back(&mut self) {
        self.current_view = View::Dashboard;
    }

    /// Get the employee for the current detail view.
    pub fn selected_employee(&self) -> Option<&Agent> {
        if let View::EmployeeDetail(ref id) = self.current_view {
            self.data.employees.iter().find(|a| &a.id == id)
        } else {
            None
        }
    }

    // --- Command bar ---

    pub fn enter_command_mode(&mut self) {
        self.command_mode = true;
        self.command_input.clear();
        self.command_cursor = 0;
    }

    pub fn exit_command_mode(&mut self) {
        self.command_mode = false;
        self.command_input.clear();
        self.command_cursor = 0;
    }

    pub fn command_insert_char(&mut self, c: char) {
        self.command_input.insert(self.command_cursor, c);
        self.command_cursor += c.len_utf8();
    }

    pub fn command_delete_char(&mut self) {
        if self.command_cursor > 0 {
            // Find the previous char boundary
            let prev = self.command_input[..self.command_cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.command_input.drain(prev..self.command_cursor);
            self.command_cursor = prev;
        }
    }

    pub fn command_move_left(&mut self) {
        if self.command_cursor > 0 {
            self.command_cursor = self.command_input[..self.command_cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn command_move_right(&mut self) {
        if self.command_cursor < self.command_input.len() {
            self.command_cursor = self.command_input[self.command_cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.command_cursor + i)
                .unwrap_or(self.command_input.len());
        }
    }

    fn push_message(&mut self, text: String, is_error: bool) {
        self.command_messages
            .push_front(CommandMessage { text, is_error });
        // Keep last 10 messages
        while self.command_messages.len() > 10 {
            self.command_messages.pop_back();
        }
    }

    /// Parse and execute the current command input.
    pub async fn execute_command(&mut self) {
        let input = self.command_input.trim().to_string();
        self.command_input.clear();
        self.command_cursor = 0;
        self.command_mode = false;

        if input.is_empty() {
            return;
        }

        let (tool_name, tool_input) = parse_command(&input);

        match tool_name {
            Some(name) => {
                let result = execute_tool(&name, &tool_input, &self.state).await;
                self.push_message(result.content, result.is_error);
                // Refresh data after mutating commands
                if name == "hire_employee" {
                    self.refresh().await;
                }
            }
            None => {
                self.push_message(format!("Unknown command: {input}"), true);
            }
        }
    }
}

/// Parse a slash command into a tool name and JSON input.
///
/// Supported commands:
///   /hire <name> <description> <team> [archetype]
///   /employees [status|team=<id>]
///   /employee <id>
///   /delegate <employee_id> <task>
///   /status
///   /teams
///   /budgets [employee_id]
fn parse_command(input: &str) -> (Option<String>, serde_json::Value) {
    let input = input.strip_prefix('/').unwrap_or(input);
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    let cmd = parts[0];
    let args = parts.get(1).unwrap_or(&"").trim();

    match cmd {
        "hire" | "h" => {
            // /hire Name "Description" team [archetype]
            let tokens = shell_split(args);
            if tokens.len() < 3 {
                return (
                    None,
                    serde_json::json!({"error": "Usage: /hire <name> <description> <team> [archetype]"}),
                );
            }
            let mut input = serde_json::json!({
                "name": tokens[0],
                "description": tokens[1],
                "team": tokens[2],
            });
            if let Some(arch) = tokens.get(3) {
                input["archetype"] = serde_json::json!(arch);
            }
            (Some("hire_employee".into()), input)
        }
        "employees" | "emp" | "ls" => {
            if args.is_empty() {
                (Some("list_employees".into()), serde_json::json!({}))
            } else if args.starts_with("team=") {
                let team = args.strip_prefix("team=").unwrap_or(args);
                (
                    Some("list_employees".into()),
                    serde_json::json!({"team": team}),
                )
            } else {
                (
                    Some("list_employees".into()),
                    serde_json::json!({"status": args}),
                )
            }
        }
        "employee" | "e" => (
            Some("get_employee".into()),
            serde_json::json!({"id": args}),
        ),
        "delegate" | "d" => {
            let tokens = shell_split(args);
            if tokens.len() < 2 {
                return (
                    None,
                    serde_json::json!({"error": "Usage: /delegate <employee_id> <task>"}),
                );
            }
            (
                Some("delegate_task".into()),
                serde_json::json!({
                    "employee_id": tokens[0],
                    "task": tokens[1..].join(" "),
                }),
            )
        }
        "status" | "s" => (Some("company_status".into()), serde_json::json!({})),
        "teams" => (Some("list_teams".into()), serde_json::json!({})),
        "budgets" | "b" => {
            if args.is_empty() {
                (Some("check_budgets".into()), serde_json::json!({}))
            } else {
                (
                    Some("check_budgets".into()),
                    serde_json::json!({"employee_id": args}),
                )
            }
        }
        _ => (None, serde_json::json!({})),
    }
}

/// Simple shell-like argument splitting that respects double quotes.
fn shell_split(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for c in input.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            ' ' if !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}
