use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
use shiioo_core::agent::AgentStatus;

use crate::tui::app::App;

/// Render the teams view showing org structure.
pub fn render_teams(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(8),   // Team list
            Constraint::Length(2), // Status bar
        ])
        .split(f.area());

    render_header(f, app, chunks[0]);
    render_team_list(f, app, chunks[1]);
    render_status_bar(f, chunks[2]);
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let team_count = app.data.teams.len();
    let total_employees = app.data.employees.len();

    let text = Line::from(vec![
        Span::styled("  Departments: ", Style::default().fg(Color::Gray)),
        Span::styled(format!("{team_count}"), Style::default().fg(Color::White)),
        Span::styled("   Employees: ", Style::default().fg(Color::Gray)),
        Span::styled(
            format!("{total_employees}"),
            Style::default().fg(Color::White),
        ),
    ]);

    let block = Block::default()
        .title(Span::styled(
            " Team Structure ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    f.render_widget(Paragraph::new(text).block(block), area);
}

fn render_team_list(f: &mut Frame, app: &App, area: Rect) {
    if app.data.teams.is_empty() {
        // No teams — show employees grouped by their team field
        let block = Block::default()
            .title(Span::styled(
                " Departments ",
                Style::default().fg(Color::Cyan),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));

        if app.data.employees.is_empty() {
            let msg = Paragraph::new(Line::from(Span::styled(
                "  No employees hired yet",
                Style::default().fg(Color::DarkGray),
            )))
            .block(block);
            f.render_widget(msg, area);
            return;
        }

        // Group employees by team
        let mut team_groups: std::collections::BTreeMap<String, Vec<&shiioo_core::agent::Agent>> =
            std::collections::BTreeMap::new();
        for emp in &app.data.employees {
            team_groups
                .entry(emp.organization.team.0.clone())
                .or_default()
                .push(emp);
        }

        let items: Vec<ListItem> = team_groups
            .iter()
            .flat_map(|(team_name, members)| {
                let mut lines = vec![ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {team_name}"),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  ({} members)", members.len()),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))];

                for emp in members {
                    let (status_label, status_color) = match emp.status {
                        AgentStatus::Active => ("active", Color::Green),
                        AgentStatus::Paused => ("on leave", Color::Yellow),
                        AgentStatus::Suspended => ("suspended", Color::Red),
                        AgentStatus::Archived => ("offboarded", Color::DarkGray),
                    };

                    let role = emp
                        .archetype
                        .as_ref()
                        .map(|a| a.0.as_str())
                        .unwrap_or("");

                    lines.push(ListItem::new(Line::from(vec![
                        Span::styled("    ", Style::default()),
                        Span::styled(&emp.name, Style::default().fg(Color::White)),
                        Span::styled(
                            format!(" ({emp_id})", emp_id = emp.id),
                            Style::default().fg(Color::DarkGray),
                        ),
                        if !role.is_empty() {
                            Span::styled(format!("  {role}"), Style::default().fg(Color::Gray))
                        } else {
                            Span::raw("")
                        },
                        Span::raw("  "),
                        Span::styled(status_label, Style::default().fg(status_color)),
                    ])));
                }

                // Blank separator between teams
                lines.push(ListItem::new(Line::from("")));
                lines
            })
            .collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
        return;
    }

    // We have registered teams — render them with lead info
    let cols = if app.data.teams.len() == 1 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(area)
    } else {
        // Split into columns, max 2
        let count = app.data.teams.len().min(2);
        let constraints: Vec<Constraint> = (0..count)
            .map(|_| Constraint::Percentage((100 / count) as u16))
            .collect();
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(area)
    };

    for (i, team) in app.data.teams.iter().enumerate() {
        if i >= cols.len() {
            break;
        }

        let mut items: Vec<ListItem> = Vec::new();

        // Team lead
        items.push(ListItem::new(Line::from(vec![
            Span::styled("  Lead: ", Style::default().fg(Color::Gray)),
            Span::styled(
                team.lead.0.clone(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ])));

        items.push(ListItem::new(Line::from("")));

        // Members
        for member_id in &team.members {
            let emp = app.data.employees.iter().find(|e| &e.id == member_id);

            let (name, status_label, status_color) = match emp {
                Some(e) => {
                    let (sl, sc) = match e.status {
                        AgentStatus::Active => ("active", Color::Green),
                        AgentStatus::Paused => ("on leave", Color::Yellow),
                        AgentStatus::Suspended => ("suspended", Color::Red),
                        AgentStatus::Archived => ("offboarded", Color::DarkGray),
                    };
                    (e.name.as_str(), sl, sc)
                }
                None => (member_id.0.as_str(), "unknown", Color::DarkGray),
            };

            items.push(ListItem::new(Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled(name, Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(status_label, Style::default().fg(status_color)),
            ])));
        }

        if team.members.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                "    (no members)",
                Style::default().fg(Color::DarkGray),
            ))));
        }

        let block = Block::default()
            .title(Span::styled(
                format!(" {} ", team.name),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));

        let list = List::new(items).block(block);
        f.render_widget(list, cols[i]);
    }
}

fn render_status_bar(f: &mut Frame, area: Rect) {
    let keys = Line::from(vec![
        Span::styled(" Esc", Style::default().fg(Color::Yellow)),
        Span::styled(" back  ", Style::default().fg(Color::DarkGray)),
        Span::styled("q", Style::default().fg(Color::Yellow)),
        Span::styled(" quit", Style::default().fg(Color::DarkGray)),
    ]);

    f.render_widget(Paragraph::new(keys), area);
}
