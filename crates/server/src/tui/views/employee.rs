use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Table},
    Frame,
};
use shiioo_core::agent::{Agent, AgentStatus, McpServerRef, SkillSource};

use crate::tui::app::App;

/// Render the employee detail view.
pub fn render_employee_detail(f: &mut Frame, app: &App) {
    let agent = match app.selected_employee() {
        Some(a) => a,
        None => {
            let msg = Paragraph::new("Employee not found.")
                .style(Style::default().fg(Color::Red));
            f.render_widget(msg, f.area());
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),  // Header / identity
            Constraint::Length(12), // Budgets
            Constraint::Min(6),    // Skills & MCP bindings
            Constraint::Length(2), // Status bar
        ])
        .split(f.area());

    render_header(f, agent, chunks[0]);
    render_budgets(f, agent, chunks[1]);
    render_skills_and_bindings(f, agent, chunks[2]);
    render_status_bar(f, chunks[3]);
}

/// Employee identity and org info.
fn render_header(f: &mut Frame, agent: &Agent, area: Rect) {
    let (status_label, status_color) = match agent.status {
        AgentStatus::Active => ("active", Color::Green),
        AgentStatus::Paused => ("on leave", Color::Yellow),
        AgentStatus::Suspended => ("suspended", Color::Red),
        AgentStatus::Archived => ("offboarded", Color::DarkGray),
    };

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Status:     ", Style::default().fg(Color::Gray)),
            Span::styled(status_label, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  Team:       ", Style::default().fg(Color::Gray)),
            Span::styled(&agent.organization.team.0, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  Reports to: ", Style::default().fg(Color::Gray)),
            Span::styled(
                agent.organization.reports_to.as_ref().map(|id| id.0.as_str()).unwrap_or("—"),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Role:       ", Style::default().fg(Color::Gray)),
            Span::styled(
                agent.archetype.as_ref().map(|a| a.0.as_str()).unwrap_or("—"),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Hired:      ", Style::default().fg(Color::Gray)),
            Span::styled(
                agent.created_at.format("%Y-%m-%d").to_string(),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];

    let title = format!(" Employee: {} ({}) ", agent.name, agent.id);
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    f.render_widget(Paragraph::new(text).block(block), area);
}

/// Budget gauges.
fn render_budgets(f: &mut Frame, agent: &Agent, area: Rect) {
    let budgets = &agent.budgets;

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Left column: tokens & cost
    let token_lines = vec![
        Line::from(""),
        budget_line("  Per request:", budgets.tokens.per_request.map(format_tokens)),
        budget_line("  Per hour:   ", budgets.tokens.per_hour.map(format_tokens)),
        budget_line("  Per day:    ", budgets.tokens.per_day.map(format_tokens)),
        budget_line("  Per month:  ", budgets.tokens.per_month.map(format_tokens)),
        Line::from(""),
        budget_line("  Cost/day:   ", budgets.cost.per_day_cents.map(|c| format_dollars(c as f64 / 100.0))),
        budget_line("  Cost/month: ", budgets.cost.per_month_cents.map(|c| format_dollars(c as f64 / 100.0))),
    ];

    let token_block = Block::default()
        .title(Span::styled(" Tokens & Cost ", Style::default().fg(Color::Cyan)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    f.render_widget(Paragraph::new(token_lines).block(token_block), cols[0]);

    // Right column: requests & concurrency
    let req_lines = vec![
        Line::from(""),
        budget_line("  Req/min:    ", budgets.requests.per_minute.map(|v| v.to_string())),
        budget_line("  Req/hour:   ", budgets.requests.per_hour.map(|v| v.to_string())),
        budget_line("  Req/day:    ", budgets.requests.per_day.map(|v| v.to_string())),
        Line::from(""),
        budget_line("  Max steps:  ", budgets.concurrency.max_concurrent_steps.map(|v| v.to_string())),
        budget_line("  Max tools:  ", budgets.concurrency.max_concurrent_tools.map(|v| v.to_string())),
        budget_line("  Max runs:   ", budgets.concurrency.max_concurrent_workflows.map(|v| v.to_string())),
    ];

    let req_block = Block::default()
        .title(Span::styled(" Requests & Concurrency ", Style::default().fg(Color::Cyan)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    f.render_widget(Paragraph::new(req_lines).block(req_block), cols[1]);
}

/// Skills and MCP bindings side by side.
fn render_skills_and_bindings(f: &mut Frame, agent: &Agent, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Skills
    let skill_rows: Vec<Row> = agent
        .skills
        .all_skills()
        .map(|s| {
            let source = match &s.source {
                SkillSource::Builtin { name } => format!("builtin:{name}"),
                SkillSource::Local { path } => format!("local:{}", path.display()),
                SkillSource::Git { repo, .. } => format!("git:{repo}"),
                SkillSource::Registry { registry, name, .. } => format!("{registry}:{name}"),
            };
            Row::new(vec![s.skill_id.clone(), source])
        })
        .collect();

    let skill_widths = [Constraint::Percentage(55), Constraint::Percentage(45)];

    let skill_header = Row::new(vec!["Skill", "Source"])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    let skills_block = Block::default()
        .title(Span::styled(" Skills ", Style::default().fg(Color::Cyan)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let skills_table = if skill_rows.is_empty() {
        Table::new(
            vec![Row::new(vec!["  (none)"])],
            [Constraint::Percentage(100)],
        )
        .block(skills_block)
    } else {
        Table::new(skill_rows, skill_widths)
            .header(skill_header)
            .block(skills_block)
    };

    f.render_widget(skills_table, cols[0]);

    // MCP Bindings
    let binding_rows: Vec<Row> = agent
        .mcp_bindings
        .iter()
        .map(|b| {
            let server = match &b.server {
                McpServerRef::Builtin => "builtin".into(),
                McpServerRef::Stdio { command, .. } => format!("stdio:{command}"),
                McpServerRef::Http { url, .. } => format!("http:{url}"),
                McpServerRef::Reference { server_id } => format!("ref:{server_id}"),
            };
            Row::new(vec![b.id.clone(), server])
        })
        .collect();

    let binding_widths = [Constraint::Percentage(40), Constraint::Percentage(60)];

    let binding_header = Row::new(vec!["Binding", "Server"])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    let bindings_block = Block::default()
        .title(Span::styled(" MCP Bindings ", Style::default().fg(Color::Cyan)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let bindings_table = if binding_rows.is_empty() {
        Table::new(
            vec![Row::new(vec!["  (none)"])],
            [Constraint::Percentage(100)],
        )
        .block(bindings_block)
    } else {
        Table::new(binding_rows, binding_widths)
            .header(binding_header)
            .block(bindings_block)
    };

    f.render_widget(bindings_table, cols[1]);
}

/// Bottom status bar.
fn render_status_bar(f: &mut Frame, area: Rect) {
    let keys = Line::from(vec![
        Span::styled(" Esc", Style::default().fg(Color::Yellow)),
        Span::styled(" back  ", Style::default().fg(Color::DarkGray)),
        Span::styled("q", Style::default().fg(Color::Yellow)),
        Span::styled(" quit", Style::default().fg(Color::DarkGray)),
    ]);

    f.render_widget(Paragraph::new(keys), area);
}

// --- helpers ---

fn budget_line<'a>(label: &'a str, value: Option<String>) -> Line<'a> {
    Line::from(vec![
        Span::styled(label, Style::default().fg(Color::Gray)),
        match value {
            Some(v) => Span::styled(v, Style::default().fg(Color::White)),
            None => Span::styled("unlimited", Style::default().fg(Color::DarkGray)),
        },
    ])
}

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn format_dollars(d: f64) -> String {
    format!("${:.2}", d)
}
