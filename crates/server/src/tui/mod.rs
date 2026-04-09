mod app;
mod event;
mod views;

use std::io;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::{
    event::KeyCode,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

use crate::config::{AppState, ServerConfig};
use app::{App, View};
use event::{AppEvent, EventHandler};
use views::{render_dashboard, render_employee_detail, render_logs, render_teams};

/// Run the TUI dashboard.
pub async fn run(config: ServerConfig, refresh_secs: u64) -> Result<()> {
    let state = AppState::new(&config).context("Failed to initialize application state")?;
    let mut app = App::new(state);

    // Initial data load
    app.refresh().await;

    // Setup terminal
    enable_raw_mode().context("Failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("Failed to enter alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("Failed to create terminal")?;

    // Install panic hook to restore terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // Event loop
    let tick_rate = Duration::from_secs(refresh_secs);
    let mut events = EventHandler::new(tick_rate);

    loop {
        // Draw based on current view
        terminal.draw(|f| match app.current_view {
            View::Dashboard => render_dashboard(f, &app),
            View::EmployeeDetail(_) => render_employee_detail(f, &app),
            View::Logs => render_logs(f, &app),
            View::Teams => render_teams(f, &app),
        })?;

        // Handle events
        if let Some(event) = events.next().await {
            match event {
                AppEvent::Key(key) => match app.current_view {
                    View::Dashboard => match key.code {
                        KeyCode::Char('q') => app.should_quit = true,
                        KeyCode::Char('j') | KeyCode::Down => app.select_next(),
                        KeyCode::Char('k') | KeyCode::Up => app.select_prev(),
                        KeyCode::Enter => app.open_employee_detail(),
                        KeyCode::Char('l') => app.current_view = View::Logs,
                        KeyCode::Char('t') => app.current_view = View::Teams,
                        KeyCode::Char('r') => app.refresh().await,
                        _ => {}
                    },
                    View::EmployeeDetail(_) => match key.code {
                        KeyCode::Esc | KeyCode::Backspace => app.go_back(),
                        KeyCode::Char('q') => app.should_quit = true,
                        _ => {}
                    },
                    View::Logs => match key.code {
                        KeyCode::Esc | KeyCode::Backspace => app.go_back(),
                        KeyCode::Char('q') => app.should_quit = true,
                        KeyCode::Char('j') | KeyCode::Down => app.log_select_next(),
                        KeyCode::Char('k') | KeyCode::Up => app.log_select_prev(),
                        _ => {}
                    },
                    View::Teams => match key.code {
                        KeyCode::Esc | KeyCode::Backspace => app.go_back(),
                        KeyCode::Char('q') => app.should_quit = true,
                        _ => {}
                    },
                },
                AppEvent::Tick => {
                    app.refresh().await;
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    // Restore terminal
    disable_raw_mode().context("Failed to disable raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("Failed to leave alternate screen")?;
    terminal.show_cursor().context("Failed to show cursor")?;

    Ok(())
}
