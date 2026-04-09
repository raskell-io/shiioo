use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::app::App;

/// Render the command bar area (replaces the status bar when command mode is active
/// or when there are recent command messages).
pub fn render_command_area(f: &mut Frame, app: &App, area: Rect) {
    if app.command_mode {
        render_command_input(f, app, area);
    } else if let Some(msg) = app.command_messages.front() {
        // Show last result + key hints
        let color = if msg.is_error {
            Color::Red
        } else {
            Color::Green
        };

        // Truncate message to fit in one line
        let max_width = area.width.saturating_sub(2) as usize;
        let truncated = if msg.text.len() > max_width {
            format!("{}...", &msg.text[..max_width.saturating_sub(3)])
        } else {
            // Only show first line if multiline
            msg.text.lines().next().unwrap_or("").to_string()
        };

        let lines = vec![
            Line::from(vec![
                Span::styled(" ", Style::default()),
                Span::styled(truncated, Style::default().fg(color)),
            ]),
            Line::from(vec![
                Span::styled(" :", Style::default().fg(Color::Yellow)),
                Span::styled(" command  ", Style::default().fg(Color::DarkGray)),
                Span::styled("q", Style::default().fg(Color::Yellow)),
                Span::styled(" quit  ", Style::default().fg(Color::DarkGray)),
                Span::styled("j/k", Style::default().fg(Color::Yellow)),
                Span::styled(" navigate  ", Style::default().fg(Color::DarkGray)),
                Span::styled("Enter", Style::default().fg(Color::Yellow)),
                Span::styled(" detail", Style::default().fg(Color::DarkGray)),
            ]),
        ];

        f.render_widget(Paragraph::new(lines), area);
    } else {
        render_default_status(f, area);
    }
}

fn render_command_input(f: &mut Frame, app: &App, area: Rect) {
    let input_line = Line::from(vec![
        Span::styled(
            " CEO > ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(&app.command_input, Style::default().fg(Color::White)),
    ]);

    let hint_line = Line::from(vec![
        Span::styled(
            " /hire /employees /employee /delegate /status /teams /budgets",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let lines = vec![input_line, hint_line];
    f.render_widget(Paragraph::new(lines), area);

    // Position cursor
    let cursor_x = area.x + 7 + app.command_cursor as u16;
    let cursor_y = area.y;
    f.set_cursor_position((cursor_x, cursor_y));
}

fn render_default_status(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(" :", Style::default().fg(Color::Yellow)),
            Span::styled(" command  ", Style::default().fg(Color::DarkGray)),
            Span::styled("q", Style::default().fg(Color::Yellow)),
            Span::styled(" quit  ", Style::default().fg(Color::DarkGray)),
            Span::styled("j/k", Style::default().fg(Color::Yellow)),
            Span::styled(" navigate  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::styled(" detail  ", Style::default().fg(Color::DarkGray)),
            Span::styled("l", Style::default().fg(Color::Yellow)),
            Span::styled(" logs  ", Style::default().fg(Color::DarkGray)),
            Span::styled("t", Style::default().fg(Color::Yellow)),
            Span::styled(" teams", Style::default().fg(Color::DarkGray)),
        ]),
    ];

    f.render_widget(Paragraph::new(lines), area);
}
