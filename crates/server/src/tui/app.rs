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
pub(crate) fn shell_split(input: &str) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use shiioo_core::agent::AgentOrganization;
    use shiioo_core::types::TeamId;

    // --- shell_split ---

    #[test]
    fn test_shell_split_simple() {
        assert_eq!(shell_split("a b c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_shell_split_quoted() {
        assert_eq!(
            shell_split(r#"Alice "Senior Engineer" engineering"#),
            vec!["Alice", "Senior Engineer", "engineering"]
        );
    }

    #[test]
    fn test_shell_split_empty() {
        assert!(shell_split("").is_empty());
    }

    #[test]
    fn test_shell_split_extra_spaces() {
        assert_eq!(shell_split("  a   b  "), vec!["a", "b"]);
    }

    #[test]
    fn test_shell_split_single_token() {
        assert_eq!(shell_split("hello"), vec!["hello"]);
    }

    #[test]
    fn test_shell_split_quoted_with_spaces() {
        assert_eq!(
            shell_split(r#""hello world""#),
            vec!["hello world"]
        );
    }

    // --- parse_command ---

    #[test]
    fn test_parse_status() {
        let (name, input) = parse_command("/status");
        assert_eq!(name.as_deref(), Some("company_status"));
        assert_eq!(input, serde_json::json!({}));
    }

    #[test]
    fn test_parse_status_alias() {
        let (name, _) = parse_command("/s");
        assert_eq!(name.as_deref(), Some("company_status"));
    }

    #[test]
    fn test_parse_teams() {
        let (name, _) = parse_command("/teams");
        assert_eq!(name.as_deref(), Some("list_teams"));
    }

    #[test]
    fn test_parse_employees_no_args() {
        let (name, input) = parse_command("/employees");
        assert_eq!(name.as_deref(), Some("list_employees"));
        assert_eq!(input, serde_json::json!({}));
    }

    #[test]
    fn test_parse_employees_by_status() {
        let (name, input) = parse_command("/employees active");
        assert_eq!(name.as_deref(), Some("list_employees"));
        assert_eq!(input["status"], "active");
    }

    #[test]
    fn test_parse_employees_by_team() {
        let (name, input) = parse_command("/employees team=engineering");
        assert_eq!(name.as_deref(), Some("list_employees"));
        assert_eq!(input["team"], "engineering");
    }

    #[test]
    fn test_parse_employees_aliases() {
        let (name, _) = parse_command("/emp");
        assert_eq!(name.as_deref(), Some("list_employees"));
        let (name, _) = parse_command("/ls");
        assert_eq!(name.as_deref(), Some("list_employees"));
    }

    #[test]
    fn test_parse_employee_detail() {
        let (name, input) = parse_command("/employee eng-alice");
        assert_eq!(name.as_deref(), Some("get_employee"));
        assert_eq!(input["id"], "eng-alice");
    }

    #[test]
    fn test_parse_hire() {
        let (name, input) = parse_command(r#"/hire Alice "Software Engineer" engineering"#);
        assert_eq!(name.as_deref(), Some("hire_employee"));
        assert_eq!(input["name"], "Alice");
        assert_eq!(input["description"], "Software Engineer");
        assert_eq!(input["team"], "engineering");
    }

    #[test]
    fn test_parse_hire_with_archetype() {
        let (name, input) = parse_command(r#"/hire Bob "QA Lead" quality qa-lead"#);
        assert_eq!(name.as_deref(), Some("hire_employee"));
        assert_eq!(input["name"], "Bob");
        assert_eq!(input["team"], "quality");
        assert_eq!(input["archetype"], "qa-lead");
    }

    #[test]
    fn test_parse_hire_alias() {
        let (name, input) = parse_command(r#"/h Charlie "Dev" eng"#);
        assert_eq!(name.as_deref(), Some("hire_employee"));
        assert_eq!(input["name"], "Charlie");
    }

    #[test]
    fn test_parse_hire_too_few_args() {
        let (name, _) = parse_command("/hire Alice");
        assert!(name.is_none()); // Returns None for insufficient args
    }

    #[test]
    fn test_parse_delegate() {
        let (name, input) = parse_command("/delegate eng-alice review the PR");
        assert_eq!(name.as_deref(), Some("delegate_task"));
        assert_eq!(input["employee_id"], "eng-alice");
        assert_eq!(input["task"], "review the PR");
    }

    #[test]
    fn test_parse_delegate_too_few_args() {
        let (name, _) = parse_command("/delegate");
        assert!(name.is_none());
    }

    #[test]
    fn test_parse_budgets_no_args() {
        let (name, input) = parse_command("/budgets");
        assert_eq!(name.as_deref(), Some("check_budgets"));
        assert_eq!(input, serde_json::json!({}));
    }

    #[test]
    fn test_parse_budgets_with_id() {
        let (name, input) = parse_command("/budgets eng-alice");
        assert_eq!(name.as_deref(), Some("check_budgets"));
        assert_eq!(input["employee_id"], "eng-alice");
    }

    #[test]
    fn test_parse_unknown_command() {
        let (name, _) = parse_command("/foobar");
        assert!(name.is_none());
    }

    #[test]
    fn test_parse_without_slash() {
        // Commands work without leading slash too
        let (name, _) = parse_command("status");
        assert_eq!(name.as_deref(), Some("company_status"));
    }

    // --- View enum ---

    #[test]
    fn test_view_equality() {
        assert_eq!(View::Dashboard, View::Dashboard);
        assert_eq!(View::Logs, View::Logs);
        assert_eq!(View::Teams, View::Teams);
        assert_ne!(View::Dashboard, View::Logs);
    }

    #[test]
    fn test_view_employee_detail_equality() {
        let id = AgentId::new("eng-alice");
        assert_eq!(
            View::EmployeeDetail(id.clone()),
            View::EmployeeDetail(id)
        );
        assert_ne!(
            View::EmployeeDetail(AgentId::new("a")),
            View::EmployeeDetail(AgentId::new("b"))
        );
    }

    // --- DashboardData & selection ---

    fn make_agent(id: &str, name: &str, status: AgentStatus) -> Agent {
        let mut agent = Agent::builder(id, name)
            .organization(AgentOrganization::new(TeamId::new("eng")))
            .build();
        agent.status = status;
        agent
    }

    fn make_data_with_employees(n: usize) -> DashboardData {
        let employees: Vec<Agent> = (0..n)
            .map(|i| make_agent(&format!("emp-{i}"), &format!("Emp {i}"), AgentStatus::Active))
            .collect();
        DashboardData {
            active_count: n,
            employees,
            ..Default::default()
        }
    }

    #[test]
    fn test_dashboard_data_default() {
        let data = DashboardData::default();
        assert!(data.employees.is_empty());
        assert_eq!(data.active_count, 0);
        assert_eq!(data.cost_24h, 0.0);
    }

    #[test]
    fn test_select_next_wraps() {
        // Can't create App without AppState, so test the logic directly on DashboardData
        let data = make_data_with_employees(3);
        let mut selected = 0usize;

        // Simulating select_next logic
        selected = (selected + 1) % data.employees.len();
        assert_eq!(selected, 1);
        selected = (selected + 1) % data.employees.len();
        assert_eq!(selected, 2);
        selected = (selected + 1) % data.employees.len();
        assert_eq!(selected, 0); // wraps
    }

    #[test]
    fn test_select_prev_wraps() {
        let data = make_data_with_employees(3);
        let mut selected = 0usize;

        // select_prev logic
        if selected == 0 {
            selected = data.employees.len() - 1;
        } else {
            selected -= 1;
        }
        assert_eq!(selected, 2); // wraps to end

        selected -= 1;
        assert_eq!(selected, 1);
    }

    #[test]
    fn test_select_on_empty_data() {
        let data = make_data_with_employees(0);
        // select_next is a no-op when empty
        assert!(data.employees.is_empty());
    }

    // --- Command input editing ---

    #[test]
    fn test_command_input_insert_and_delete() {
        let mut input = String::new();
        let mut cursor = 0usize;

        // Insert chars
        input.insert(cursor, 'a');
        cursor += 1;
        input.insert(cursor, 'b');
        cursor += 1;
        input.insert(cursor, 'c');
        cursor += 1;
        assert_eq!(input, "abc");
        assert_eq!(cursor, 3);

        // Delete last char (backspace logic from command_delete_char)
        if cursor > 0 {
            let prev = input[..cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            input.drain(prev..cursor);
            cursor = prev;
        }
        assert_eq!(input, "ab");
        assert_eq!(cursor, 2);
    }

    #[test]
    fn test_command_input_cursor_movement() {
        let input = "hello";
        let mut cursor = input.len(); // at end

        // Move left
        cursor = input[..cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        assert_eq!(cursor, 4); // before 'o'

        // Move right
        cursor = input[cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| cursor + i)
            .unwrap_or(input.len());
        assert_eq!(cursor, 5); // back to end
    }

    #[test]
    fn test_command_input_cursor_at_boundaries() {
        let input = "ab";
        let cursor = 0usize;

        // Move left at 0 stays at 0
        let new_cursor = if cursor > 0 {
            input[..cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0)
        } else {
            0
        };
        assert_eq!(new_cursor, 0);

        // Move right at end stays at end
        let cursor = input.len();
        let new_cursor = input[cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| cursor + i)
            .unwrap_or(input.len());
        assert_eq!(new_cursor, input.len());
    }

    // --- CommandMessage ---

    #[test]
    fn test_push_message_limits_to_10() {
        let mut messages: VecDeque<CommandMessage> = VecDeque::new();
        for i in 0..15 {
            messages.push_front(CommandMessage {
                text: format!("msg {i}"),
                is_error: false,
            });
            while messages.len() > 10 {
                messages.pop_back();
            }
        }
        assert_eq!(messages.len(), 10);
        assert_eq!(messages.front().unwrap().text, "msg 14");
        assert_eq!(messages.back().unwrap().text, "msg 5");
    }

    // --- Employee lookup ---

    #[test]
    fn test_selected_employee_on_dashboard_view() {
        // selected_employee returns None when not on EmployeeDetail view
        let data = make_data_with_employees(3);
        let view = View::Dashboard;
        let result = if let View::EmployeeDetail(ref id) = view {
            data.employees.iter().find(|a| &a.id == id)
        } else {
            None
        };
        assert!(result.is_none());
    }

    #[test]
    fn test_selected_employee_found() {
        let data = make_data_with_employees(3);
        let target_id = data.employees[1].id.clone();
        let view = View::EmployeeDetail(target_id.clone());

        let result = if let View::EmployeeDetail(ref id) = view {
            data.employees.iter().find(|a| &a.id == id)
        } else {
            None
        };
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, target_id);
    }

    #[test]
    fn test_selected_employee_not_found() {
        let data = make_data_with_employees(3);
        let view = View::EmployeeDetail(AgentId::new("nonexistent"));

        let result = if let View::EmployeeDetail(ref id) = view {
            data.employees.iter().find(|a| &a.id == id)
        } else {
            None
        };
        assert!(result.is_none());
    }
}
