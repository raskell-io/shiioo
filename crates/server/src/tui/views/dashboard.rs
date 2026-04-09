use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};
use shiioo_core::agent::AgentStatus;

use crate::tui::app::App;

/// Render the main CEO dashboard.
pub fn render_dashboard(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9),  // Company overview
            Constraint::Min(10),   // Employee table
            Constraint::Length(2), // Status bar
        ])
        .split(f.area());

    render_overview(f, app, chunks[0]);
    render_employee_table(f, app, chunks[1]);
    render_status_bar(f, chunks[2]);
}

/// Company overview panel.
fn render_overview(f: &mut Frame, app: &App, area: Rect) {
    let data = &app.data;
    let total = data.employees.len();

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Employees: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{} active", data.active_count),
                Style::default().fg(Color::Green),
            ),
            Span::raw(" / "),
            Span::styled(format!("{total} total"), Style::default().fg(Color::White)),
            if data.paused_count > 0 {
                Span::styled(
                    format!("  ({} on leave)", data.paused_count),
                    Style::default().fg(Color::Yellow),
                )
            } else {
                Span::raw("")
            },
            if data.suspended_count > 0 {
                Span::styled(
                    format!("  ({} suspended)", data.suspended_count),
                    Style::default().fg(Color::Red),
                )
            } else {
                Span::raw("")
            },
        ]),
        Line::from(vec![
            Span::styled("  Teams:     ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{}", data.team_count),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Approvals: ", Style::default().fg(Color::Gray)),
            if data.pending_approvals > 0 {
                Span::styled(
                    format!("{} pending", data.pending_approvals),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled("none", Style::default().fg(Color::DarkGray))
            },
        ]),
        Line::from(vec![
            Span::styled("  LLM:       ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{} sources", data.capacity_sources),
                Style::default().fg(Color::White),
            ),
            Span::raw("  "),
            Span::styled("Cost (24h): ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("${:.2}", data.cost_24h),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(""),
    ];

    let block = Block::default()
        .title(Span::styled(
            " Shiioo — CEO Command Center ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(text).block(block);
    f.render_widget(paragraph, area);
}

/// Employee table.
fn render_employee_table(f: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(vec![
        Cell::from("Name"),
        Cell::from("Status"),
        Cell::from("Team"),
        Cell::from("Role"),
        Cell::from("Reports To"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
    .height(1);

    let rows: Vec<Row> = app
        .data
        .employees
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let status_style = match agent.status {
                AgentStatus::Active => Style::default().fg(Color::Green),
                AgentStatus::Paused => Style::default().fg(Color::Yellow),
                AgentStatus::Suspended => Style::default().fg(Color::Red),
                AgentStatus::Archived => Style::default().fg(Color::DarkGray),
            };

            let status_label = match agent.status {
                AgentStatus::Active => "active",
                AgentStatus::Paused => "on leave",
                AgentStatus::Suspended => "suspended",
                AgentStatus::Archived => "offboarded",
            };

            let row_style = if i == app.selected {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };

            Row::new(vec![
                Cell::from(agent.name.clone()),
                Cell::from(Span::styled(status_label, status_style)),
                Cell::from(agent.organization.team.0.clone()),
                Cell::from(
                    agent
                        .archetype
                        .as_ref()
                        .map(|a| a.0.clone())
                        .unwrap_or_default(),
                ),
                Cell::from(
                    agent
                        .organization
                        .reports_to
                        .as_ref()
                        .map(|id| id.0.clone())
                        .unwrap_or_else(|| "—".into()),
                ),
            ])
            .style(row_style)
        })
        .collect();

    let block = Block::default()
        .title(Span::styled(
            " Employees ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let widths = [
        Constraint::Percentage(20),
        Constraint::Percentage(12),
        Constraint::Percentage(20),
        Constraint::Percentage(25),
        Constraint::Percentage(23),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(
            Style::default()
                .add_modifier(Modifier::REVERSED)
                .fg(Color::Cyan),
        );

    let mut table_state = TableState::default();
    if !app.data.employees.is_empty() {
        table_state.select(Some(app.selected));
    }

    f.render_stateful_widget(table, area, &mut table_state);
}

/// Bottom status bar with key bindings.
fn render_status_bar(f: &mut Frame, area: Rect) {
    let keys = Line::from(vec![
        Span::styled(" q", Style::default().fg(Color::Yellow)),
        Span::styled(" quit  ", Style::default().fg(Color::DarkGray)),
        Span::styled("j/k", Style::default().fg(Color::Yellow)),
        Span::styled(" navigate  ", Style::default().fg(Color::DarkGray)),
        Span::styled("r", Style::default().fg(Color::Yellow)),
        Span::styled(" refresh", Style::default().fg(Color::DarkGray)),
    ]);

    let paragraph = Paragraph::new(keys);
    f.render_widget(paragraph, area);
}
