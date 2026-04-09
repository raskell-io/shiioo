use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};
use shiioo_core::analytics::TraceStatus;

use crate::tui::app::App;

/// Render the full-screen log viewer.
pub fn render_logs(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(8),   // Log table
            Constraint::Length(2), // Status bar
        ])
        .split(f.area());

    render_header(f, app, chunks[0]);
    render_log_table(f, app, chunks[1]);
    render_status_bar(f, chunks[2]);
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let total = app.data.recent_traces.len();
    let running = app
        .data
        .recent_traces
        .iter()
        .filter(|t| t.status == TraceStatus::Running)
        .count();
    let failed = app
        .data
        .recent_traces
        .iter()
        .filter(|t| t.status == TraceStatus::Failed)
        .count();

    let text = Line::from(vec![
        Span::styled("  Traces: ", Style::default().fg(Color::Gray)),
        Span::styled(format!("{total}"), Style::default().fg(Color::White)),
        if running > 0 {
            Span::styled(
                format!("  ({running} running)"),
                Style::default().fg(Color::Blue),
            )
        } else {
            Span::raw("")
        },
        if failed > 0 {
            Span::styled(
                format!("  ({failed} failed)"),
                Style::default().fg(Color::Red),
            )
        } else {
            Span::raw("")
        },
    ]);

    let block = Block::default()
        .title(Span::styled(
            " Event Log ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    f.render_widget(Paragraph::new(text).block(block), area);
}

fn render_log_table(f: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(vec![
        Cell::from("Time"),
        Cell::from("Status"),
        Cell::from("Workflow"),
        Cell::from("Run ID"),
        Cell::from("Duration"),
        Cell::from("Steps"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
    .height(1);

    let rows: Vec<Row> = app
        .data
        .recent_traces
        .iter()
        .enumerate()
        .map(|(i, trace)| {
            let (status_label, status_color) = match trace.status {
                TraceStatus::Running => ("running", Color::Blue),
                TraceStatus::Completed => ("done", Color::Green),
                TraceStatus::Failed => ("FAILED", Color::Red),
                TraceStatus::Cancelled => ("cancel", Color::DarkGray),
            };

            let duration = trace
                .duration_secs
                .map(|d| {
                    if d < 1.0 {
                        format!("{:.0}ms", d * 1000.0)
                    } else if d < 60.0 {
                        format!("{:.1}s", d)
                    } else {
                        format!("{:.1}m", d / 60.0)
                    }
                })
                .unwrap_or_else(|| "...".into());

            let step_summary = format!(
                "{}/{}",
                trace
                    .steps
                    .iter()
                    .filter(|s| s.status == TraceStatus::Completed)
                    .count(),
                trace.steps.len()
            );

            let row_style = if i == app.log_selected {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };

            Row::new(vec![
                Cell::from(trace.started_at.format("%H:%M:%S").to_string()),
                Cell::from(Span::styled(status_label, Style::default().fg(status_color))),
                Cell::from(trace.workflow_id.clone()),
                Cell::from(trace.run_id.0.to_string()),
                Cell::from(duration),
                Cell::from(step_summary),
            ])
            .style(row_style)
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let widths = [
        Constraint::Length(10),
        Constraint::Length(9),
        Constraint::Percentage(25),
        Constraint::Percentage(30),
        Constraint::Length(9),
        Constraint::Length(7),
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
    if !app.data.recent_traces.is_empty() {
        table_state.select(Some(app.log_selected));
    }

    f.render_stateful_widget(table, area, &mut table_state);
}

fn render_status_bar(f: &mut Frame, area: Rect) {
    let keys = Line::from(vec![
        Span::styled(" Esc", Style::default().fg(Color::Yellow)),
        Span::styled(" back  ", Style::default().fg(Color::DarkGray)),
        Span::styled("j/k", Style::default().fg(Color::Yellow)),
        Span::styled(" scroll  ", Style::default().fg(Color::DarkGray)),
        Span::styled("q", Style::default().fg(Color::Yellow)),
        Span::styled(" quit", Style::default().fg(Color::DarkGray)),
    ]);

    f.render_widget(Paragraph::new(keys), area);
}
