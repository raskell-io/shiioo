use shiioo_core::agent::{Agent, AgentId, AgentStatus};
use shiioo_core::storage::AgentStore;
use shiioo_core::types::ApprovalStatus;

use crate::config::AppState;

/// Which screen is currently displayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Dashboard,
    EmployeeDetail(AgentId),
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
}

/// Main TUI application state.
pub struct App {
    pub state: AppState,
    pub current_view: View,
    pub data: DashboardData,
    pub selected: usize,
    pub should_quit: bool,
}

impl App {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            current_view: View::Dashboard,
            data: DashboardData::default(),
            selected: 0,
            should_quit: false,
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

        // Clamp selection
        if !self.data.employees.is_empty() && self.selected >= self.data.employees.len() {
            self.selected = self.data.employees.len() - 1;
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
}
