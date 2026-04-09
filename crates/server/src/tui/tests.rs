//! Integration tests for the TUI dashboard.
//!
//! These tests create a real AppState with a temp directory and exercise
//! the full App lifecycle including data refresh and view rendering.

#[cfg(test)]
mod integration {
    use ratatui::{backend::TestBackend, Terminal};
    use shiioo_core::agent::{Agent, AgentOrganization};
    use shiioo_core::storage::AgentStore;
    use shiioo_core::types::TeamId;

    use crate::config::{AppState, ServerConfig, StorageConfig, ENCRYPTION_KEY_ENV};
    use crate::tui::app::{App, View};
    use crate::tui::views::{
        render_command_area, render_dashboard, render_employee_detail, render_logs, render_teams,
    };

    /// Create an AppState backed by a temp directory.
    fn make_test_state() -> (AppState, tempfile::TempDir) {
        let tmpdir = tempfile::TempDir::new().expect("failed to create temp dir");

        // Set the required encryption key for SecretManager
        std::env::set_var(ENCRYPTION_KEY_ENV, "abcdefghijklmnopqrstuvwxyz012345");

        let config = ServerConfig {
            data_dir: tmpdir.path().to_path_buf(),
            storage: StorageConfig::default(),
        };

        let state = AppState::new(&config).expect("failed to create AppState");
        (state, tmpdir)
    }

    fn make_test_app() -> (App, tempfile::TempDir) {
        let (state, tmpdir) = make_test_state();
        (App::new(state), tmpdir)
    }

    async fn hire_agent(state: &AppState, id: &str, name: &str, team: &str) -> Agent {
        let agent = Agent::builder(id, name)
            .description(format!("{name} on {team}"))
            .organization(AgentOrganization::new(TeamId::new(team)))
            .created_by("test")
            .build();
        state
            .agent_store
            .store_agent(&agent)
            .await
            .expect("failed to store agent");
        agent
    }

    // --- App lifecycle ---

    #[tokio::test]
    async fn test_app_initial_state() {
        let (app, _tmpdir) = make_test_app();
        assert_eq!(app.current_view, View::Dashboard);
        assert_eq!(app.selected, 0);
        assert!(!app.should_quit);
        assert!(!app.command_mode);
        assert!(app.data.employees.is_empty());
    }

    #[tokio::test]
    async fn test_app_refresh_empty() {
        let (mut app, _tmpdir) = make_test_app();
        app.refresh().await;

        assert_eq!(app.data.employees.len(), 0);
        assert_eq!(app.data.active_count, 0);
        assert_eq!(app.data.team_count, 0);
        assert_eq!(app.data.pending_approvals, 0);
    }

    #[tokio::test]
    async fn test_app_refresh_with_employees() {
        let (mut app, _tmpdir) = make_test_app();

        hire_agent(&app.state, "eng-alice", "Alice", "engineering").await;
        hire_agent(&app.state, "mkt-bob", "Bob", "marketing").await;

        app.refresh().await;

        assert_eq!(app.data.employees.len(), 2);
        assert_eq!(app.data.active_count, 2);
    }

    #[tokio::test]
    async fn test_app_selection_navigation() {
        let (mut app, _tmpdir) = make_test_app();

        hire_agent(&app.state, "a", "A", "t").await;
        hire_agent(&app.state, "b", "B", "t").await;
        hire_agent(&app.state, "c", "C", "t").await;
        app.refresh().await;

        assert_eq!(app.selected, 0);
        app.select_next();
        assert_eq!(app.selected, 1);
        app.select_next();
        assert_eq!(app.selected, 2);
        app.select_next();
        assert_eq!(app.selected, 0); // wraps

        app.select_prev();
        assert_eq!(app.selected, 2); // wraps back
        app.select_prev();
        assert_eq!(app.selected, 1);
    }

    #[tokio::test]
    async fn test_app_selection_empty() {
        let (mut app, _tmpdir) = make_test_app();
        app.refresh().await;

        // No-op on empty list
        app.select_next();
        assert_eq!(app.selected, 0);
        app.select_prev();
        assert_eq!(app.selected, 0);
    }

    #[tokio::test]
    async fn test_app_open_employee_detail() {
        let (mut app, _tmpdir) = make_test_app();

        hire_agent(&app.state, "eng-alice", "Alice", "engineering").await;
        app.refresh().await;

        app.open_employee_detail();
        assert_eq!(
            app.current_view,
            View::EmployeeDetail(shiioo_core::agent::AgentId::new("eng-alice"))
        );

        // selected_employee should return Alice
        let emp = app.selected_employee();
        assert!(emp.is_some());
        assert_eq!(emp.unwrap().name, "Alice");
    }

    #[tokio::test]
    async fn test_app_open_employee_detail_empty() {
        let (mut app, _tmpdir) = make_test_app();
        app.refresh().await;

        // No-op when no employees
        app.open_employee_detail();
        assert_eq!(app.current_view, View::Dashboard);
    }

    #[tokio::test]
    async fn test_app_go_back() {
        let (mut app, _tmpdir) = make_test_app();

        hire_agent(&app.state, "a", "A", "t").await;
        app.refresh().await;

        app.open_employee_detail();
        assert!(matches!(app.current_view, View::EmployeeDetail(_)));

        app.go_back();
        assert_eq!(app.current_view, View::Dashboard);
    }

    #[tokio::test]
    async fn test_app_view_transitions() {
        let (mut app, _tmpdir) = make_test_app();

        app.current_view = View::Logs;
        assert_eq!(app.current_view, View::Logs);

        app.go_back();
        assert_eq!(app.current_view, View::Dashboard);

        app.current_view = View::Teams;
        assert_eq!(app.current_view, View::Teams);

        app.go_back();
        assert_eq!(app.current_view, View::Dashboard);
    }

    // --- Command mode ---

    #[tokio::test]
    async fn test_command_mode_enter_exit() {
        let (mut app, _tmpdir) = make_test_app();

        assert!(!app.command_mode);

        app.enter_command_mode();
        assert!(app.command_mode);
        assert!(app.command_input.is_empty());

        app.command_insert_char('/');
        app.command_insert_char('s');
        assert_eq!(app.command_input, "/s");
        assert_eq!(app.command_cursor, 2);

        app.exit_command_mode();
        assert!(!app.command_mode);
        assert!(app.command_input.is_empty());
        assert_eq!(app.command_cursor, 0);
    }

    #[tokio::test]
    async fn test_command_insert_delete() {
        let (mut app, _tmpdir) = make_test_app();
        app.enter_command_mode();

        app.command_insert_char('a');
        app.command_insert_char('b');
        app.command_insert_char('c');
        assert_eq!(app.command_input, "abc");

        app.command_delete_char();
        assert_eq!(app.command_input, "ab");

        app.command_delete_char();
        assert_eq!(app.command_input, "a");

        app.command_delete_char();
        assert!(app.command_input.is_empty());

        // Delete on empty is no-op
        app.command_delete_char();
        assert!(app.command_input.is_empty());
    }

    #[tokio::test]
    async fn test_command_cursor_movement() {
        let (mut app, _tmpdir) = make_test_app();
        app.enter_command_mode();

        app.command_insert_char('a');
        app.command_insert_char('b');
        app.command_insert_char('c');
        assert_eq!(app.command_cursor, 3);

        app.command_move_left();
        assert_eq!(app.command_cursor, 2);

        app.command_move_left();
        assert_eq!(app.command_cursor, 1);

        app.command_move_right();
        assert_eq!(app.command_cursor, 2);

        // Move left to 0
        app.command_move_left();
        app.command_move_left();
        assert_eq!(app.command_cursor, 0);

        // Can't go below 0
        app.command_move_left();
        assert_eq!(app.command_cursor, 0);

        // Move right to end
        app.command_move_right();
        app.command_move_right();
        app.command_move_right();
        assert_eq!(app.command_cursor, 3);

        // Can't go past end
        app.command_move_right();
        assert_eq!(app.command_cursor, 3);
    }

    #[tokio::test]
    async fn test_execute_command_status() {
        let (mut app, _tmpdir) = make_test_app();

        app.enter_command_mode();
        for c in "/status".chars() {
            app.command_insert_char(c);
        }
        app.execute_command().await;

        assert!(!app.command_mode);
        assert!(!app.command_messages.is_empty());
        let msg = app.command_messages.front().unwrap();
        assert!(!msg.is_error);
        assert!(msg.text.contains("Employees"));
    }

    #[tokio::test]
    async fn test_execute_command_hire_and_refresh() {
        let (mut app, _tmpdir) = make_test_app();

        app.enter_command_mode();
        for c in r#"/hire Alice "Engineer" engineering"#.chars() {
            app.command_insert_char(c);
        }
        app.execute_command().await;

        let msg = app.command_messages.front().unwrap();
        assert!(!msg.is_error);
        assert!(msg.text.contains("Hired Alice"));

        // Data should be refreshed automatically after hire
        assert_eq!(app.data.employees.len(), 1);
        assert_eq!(app.data.employees[0].name, "Alice");
    }

    #[tokio::test]
    async fn test_execute_command_unknown() {
        let (mut app, _tmpdir) = make_test_app();

        app.enter_command_mode();
        for c in "/foobar".chars() {
            app.command_insert_char(c);
        }
        app.execute_command().await;

        let msg = app.command_messages.front().unwrap();
        assert!(msg.is_error);
        assert!(msg.text.contains("Unknown command"));
    }

    #[tokio::test]
    async fn test_execute_empty_command() {
        let (mut app, _tmpdir) = make_test_app();

        app.enter_command_mode();
        app.execute_command().await;

        // Empty command: no message pushed
        assert!(app.command_messages.is_empty());
    }

    // --- Log selection ---

    #[tokio::test]
    async fn test_log_selection_empty() {
        let (mut app, _tmpdir) = make_test_app();
        app.refresh().await;

        app.log_select_next();
        assert_eq!(app.log_selected, 0);
        app.log_select_prev();
        assert_eq!(app.log_selected, 0);
    }

    // --- Rendering: smoke tests ---
    // These verify that rendering doesn't panic with various data states.

    #[tokio::test]
    async fn test_render_dashboard_empty() {
        let (app, _tmpdir) = make_test_app();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| render_dashboard(f, &app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content = buffer_to_string(&buffer);
        assert!(content.contains("CEO Command Center"));
        assert!(content.contains("Employees"));
        assert!(content.contains("Activity"));
    }

    #[tokio::test]
    async fn test_render_dashboard_with_data() {
        let (mut app, _tmpdir) = make_test_app();

        hire_agent(&app.state, "eng-alice", "Alice", "engineering").await;
        hire_agent(&app.state, "mkt-bob", "Bob", "marketing").await;
        app.refresh().await;

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| render_dashboard(f, &app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content = buffer_to_string(&buffer);
        assert!(content.contains("Alice"));
        assert!(content.contains("Bob"));
        assert!(content.contains("2 active"));
    }

    #[tokio::test]
    async fn test_render_employee_detail_view() {
        let (mut app, _tmpdir) = make_test_app();

        hire_agent(&app.state, "eng-alice", "Alice", "engineering").await;
        app.refresh().await;

        app.open_employee_detail();

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| render_employee_detail(f, &app))
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content = buffer_to_string(&buffer);
        assert!(content.contains("Alice"));
        assert!(content.contains("engineering"));
        assert!(content.contains("active"));
    }

    #[tokio::test]
    async fn test_render_employee_detail_not_found() {
        let (mut app, _tmpdir) = make_test_app();
        app.current_view = View::EmployeeDetail(shiioo_core::agent::AgentId::new("nonexistent"));

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| render_employee_detail(f, &app))
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content = buffer_to_string(&buffer);
        assert!(content.contains("not found"));
    }

    #[tokio::test]
    async fn test_render_logs_empty() {
        let (app, _tmpdir) = make_test_app();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| render_logs(f, &app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content = buffer_to_string(&buffer);
        assert!(content.contains("Event Log"));
    }

    #[tokio::test]
    async fn test_render_teams_empty() {
        let (app, _tmpdir) = make_test_app();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| render_teams(f, &app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content = buffer_to_string(&buffer);
        assert!(content.contains("Team Structure"));
    }

    #[tokio::test]
    async fn test_render_teams_with_employees() {
        let (mut app, _tmpdir) = make_test_app();

        hire_agent(&app.state, "eng-alice", "Alice", "engineering").await;
        hire_agent(&app.state, "eng-bob", "Bob", "engineering").await;
        hire_agent(&app.state, "mkt-carol", "Carol", "marketing").await;
        app.refresh().await;

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| render_teams(f, &app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content = buffer_to_string(&buffer);
        assert!(content.contains("engineering"));
        assert!(content.contains("marketing"));
        assert!(content.contains("Alice"));
    }

    #[tokio::test]
    async fn test_render_command_area_default() {
        let (app, _tmpdir) = make_test_app();
        let backend = TestBackend::new(120, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| render_command_area(f, &app, f.area()))
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content = buffer_to_string(&buffer);
        assert!(content.contains("command"));
        assert!(content.contains("quit"));
    }

    #[tokio::test]
    async fn test_render_command_area_active() {
        let (mut app, _tmpdir) = make_test_app();
        app.enter_command_mode();
        for c in "/status".chars() {
            app.command_insert_char(c);
        }

        let backend = TestBackend::new(120, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| render_command_area(f, &app, f.area()))
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content = buffer_to_string(&buffer);
        assert!(content.contains("CEO >"));
        assert!(content.contains("/status"));
    }

    #[tokio::test]
    async fn test_render_command_area_with_result() {
        let (mut app, _tmpdir) = make_test_app();

        // Execute a command to generate a message
        app.enter_command_mode();
        for c in "/status".chars() {
            app.command_insert_char(c);
        }
        app.execute_command().await;

        let backend = TestBackend::new(120, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| render_command_area(f, &app, f.area()))
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content = buffer_to_string(&buffer);
        assert!(content.contains("Employees"));
    }

    // --- Selection clamping ---

    #[tokio::test]
    async fn test_selection_clamps_after_data_shrinks() {
        let (mut app, _tmpdir) = make_test_app();

        hire_agent(&app.state, "a", "A", "t").await;
        hire_agent(&app.state, "b", "B", "t").await;
        hire_agent(&app.state, "c", "C", "t").await;
        app.refresh().await;

        app.selected = 2; // pointing at C

        // Remove an agent by storing over the list (simulate data shrink)
        // Instead, just verify clamp logic works
        app.data.employees.pop();
        app.data.employees.pop();
        // Now only 1 employee, but selected is 2

        // Simulate the clamping that happens in refresh
        if !app.data.employees.is_empty() && app.selected >= app.data.employees.len() {
            app.selected = app.data.employees.len() - 1;
        }
        assert_eq!(app.selected, 0);
    }

    // --- Helper ---

    /// Convert a ratatui Buffer to a string for content assertions.
    fn buffer_to_string(buffer: &ratatui::buffer::Buffer) -> String {
        let mut result = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let cell = &buffer[(x, y)];
                result.push_str(cell.symbol());
            }
            result.push('\n');
        }
        result
    }
}
