# TUI Dashboard

> The CEO Command Center -- a terminal-based dashboard for monitoring and managing your virtual company.

## Quick Start

```bash
# Launch the dashboard
export SHIIOO_ENCRYPTION_KEY=$(openssl rand -base64 32 | head -c 32)
shiioo dashboard

# Custom refresh interval (default: 2 seconds)
shiioo dashboard --refresh 5
```

The dashboard runs in an alternate terminal screen. Press `q` to quit and restore your terminal.

---

## Views

The dashboard has four views, each accessible via a single keypress from the main dashboard.

### 1. Dashboard (default)

The main screen showing a company overview, employee table, and live activity feed.

```
+-- Shiioo -- CEO Command Center ----------------------------------------+
|  Employees: 3 active / 5 total  (1 on leave)                          |
|  Teams:     2    Approvals: 1 pending                                  |
|  LLM:       2 sources  Cost (24h): $12.45                              |
+-- Employees ---------------------+-- Activity -------------------------+
| Name     Status  Team    Role    | 10:23:45 ok data-pipeline (2.1s)    |
| > Alice  active  eng     SWE    | 10:21:12 >> pr-review               |
|   Bob    active  mkt     PM     | 10:20:00 !! deploy (45.0s)          |
|   Charlie on leave qa            | 10:18:30 ok code-lint (312ms)       |
+----------------------------------+-  --  --  --  --  --  --  --  --  --+
| : command  q quit  j/k navigate  Enter detail  l logs  t teams         |
+------------------------------------------------------------------------+
```

**Company Overview** shows:
- Employee counts by status (active, on leave, suspended)
- Number of teams
- Pending approval count (highlighted in yellow when > 0)
- LLM capacity source count and rolling 24-hour cost

**Employee Table** shows Name, Status, Team, and Role for each employee. The selected row is highlighted.

**Activity Feed** shows recent execution traces with:
- Timestamp
- Status indicator: `ok` (completed), `>>` (running), `!!` (failed), `--` (cancelled)
- Workflow ID
- Duration

### 2. Employee Detail (`Enter`)

Press `Enter` on a selected employee to drill into their profile.

```
+-- Employee: Alice (eng-alice) -----------------------------------------+
|  Status:     active                                                    |
|  Team:       engineering                                               |
|  Reports to: lead-bob                                                  |
|  Role:       software-engineer                                         |
|  Hired:      2026-03-15                                                |
+-- Tokens & Cost ---------------------+-- Requests & Concurrency -------+
|  Per request: 100K                   |  Req/min:    30                  |
|  Per hour:    1.0M                   |  Req/hour:   unlimited           |
|  Per day:     5.0M                   |  Req/day:    unlimited           |
|  Per month:   unlimited              |                                  |
|                                      |  Max steps:  5                   |
|  Cost/day:    $50.00                 |  Max tools:  10                  |
|  Cost/month:  $1000.00               |  Max runs:   3                   |
+-- Skills -------------------------+-- MCP Bindings --------------------+
| code-review     builtin:code-rev  | github      stdio:gh-mcp           |
| debugging       git:debug-repo    | jira        http:jira.internal      |
+-----------------------------------+------------------------------------+
| Esc back  q quit                                                       |
+------------------------------------------------------------------------+
```

Shows:
- Identity and org placement (team, supervisor, role, hire date)
- Budget limits across four dimensions: tokens, cost, requests, concurrency
- All skills with their source type (builtin, local, git, registry)
- MCP tool bindings with server type (builtin, stdio, http, reference)

### 3. Event Log (`l`)

Full-screen scrollable log of execution traces.

```
+-- Event Log -----------------------------------------------------------+
|  Traces: 12  (1 running)  (2 failed)                                  |
+------------------------------------------------------------------------+
| Time      Status   Workflow           Run ID           Duration  Steps |
| 10:23:45  done     data-pipeline      a1b2c3d4-...     2.1s     3/3   |
| 10:21:12  running  pr-review          e5f6a7b8-...     ...      1/4   |
| 10:20:00  FAILED   deploy             c9d0e1f2-...     45.0s    2/5   |
| 10:18:30  done     code-lint          a3b4c5d6-...     312ms    1/1   |
+------------------------------------------------------------------------+
| Esc back  j/k scroll  q quit                                          |
+------------------------------------------------------------------------+
```

Columns:
- **Time** -- when the trace started
- **Status** -- done, running, FAILED, cancel
- **Workflow** -- the workflow identifier
- **Run ID** -- unique execution ID
- **Duration** -- total elapsed time
- **Steps** -- completed/total step count

### 4. Teams (`t`)

Organizational view showing employees grouped by department.

```
+-- Team Structure ------------------------------------------------------+
|  Departments: 3   Employees: 8                                        |
+-- Departments ---------------------------------------------------------+
|  engineering  (3 members)                                              |
|    Alice (eng-alice)  software-engineer  active                        |
|    Bob (eng-bob)  software-engineer  active                            |
|    Dave (eng-dave)  devops  on leave                                   |
|                                                                        |
|  marketing  (2 members)                                                |
|    Carol (mkt-carol)  pm  active                                       |
|    Eve (mkt-eve)  designer  active                                     |
|                                                                        |
|  qa  (1 member)                                                        |
|    Frank (qa-frank)  qa-engineer  suspended                            |
+------------------------------------------------------------------------+
| Esc back  q quit                                                       |
+------------------------------------------------------------------------+
```

If registered teams exist (via the orchestrator), the view shows team lead and members. Otherwise, it groups employees by their team field.

---

## Command Bar

Press `:` or `/` on the dashboard to activate the command bar. This provides direct access to company operations without leaving the TUI.

```
+------------------------------------------------------------------------+
| CEO > /hire Alice "Senior Engineer" engineering                        |
| /hire /employees /employee /delegate /status /teams /budgets           |
+------------------------------------------------------------------------+
```

### Available Commands

| Command | Alias | Description | Example |
|---|---|---|---|
| `/hire <name> <desc> <team> [role]` | `/h` | Hire a new employee | `/hire Alice "Senior Engineer" engineering swe` |
| `/employees [filter]` | `/emp`, `/ls` | List employees | `/employees active` or `/employees team=engineering` |
| `/employee <id>` | `/e` | Get employee details | `/employee eng-alice` |
| `/delegate <id> <task>` | `/d` | Delegate a task | `/delegate eng-alice review the login PR` |
| `/status` | `/s` | Company overview | `/status` |
| `/teams` | | List all teams | `/teams` |
| `/budgets [id]` | `/b` | Check budgets | `/budgets` or `/budgets eng-alice` |

**Notes:**
- Use double quotes for multi-word arguments: `/hire Alice "Frontend Engineer" engineering`
- The `/` prefix is optional -- `status` and `/status` both work
- Results appear inline with green (success) or red (error) highlighting
- After `/hire`, the employee table refreshes automatically

### Command Mode Keys

| Key | Action |
|---|---|
| `Enter` | Execute the command |
| `Esc` | Cancel and exit command mode |
| `Backspace` | Delete character before cursor |
| `Left` / `Right` | Move cursor within input |

---

## Key Bindings

### Dashboard

| Key | Action |
|---|---|
| `j` / `Down` | Select next employee |
| `k` / `Up` | Select previous employee |
| `Enter` | Open employee detail |
| `l` | Open event log |
| `t` | Open teams view |
| `:` | Open command bar |
| `/` | Open command bar (pre-fills `/`) |
| `r` | Force refresh data |
| `q` | Quit |

### Employee Detail

| Key | Action |
|---|---|
| `Esc` / `Backspace` | Back to dashboard |
| `q` | Quit |

### Event Log

| Key | Action |
|---|---|
| `j` / `Down` | Select next trace |
| `k` / `Up` | Select previous trace |
| `Esc` / `Backspace` | Back to dashboard |
| `q` | Quit |

### Teams

| Key | Action |
|---|---|
| `Esc` / `Backspace` | Back to dashboard |
| `q` | Quit |

---

## Architecture

### Module Structure

```
crates/server/src/tui/
  mod.rs          -- Entry point, terminal setup, main event loop
  app.rs          -- App state, data cache, command parsing, view transitions
  event.rs        -- Crossterm event polling with async tick timer
  tests.rs        -- Unit and integration tests
  views/
    mod.rs        -- View module exports
    dashboard.rs  -- Main dashboard (overview + employee table + activity feed)
    employee.rs   -- Employee detail view (budgets, skills, MCP bindings)
    logs.rs       -- Full-screen execution trace log viewer
    teams.rs      -- Org structure / department grouping view
    command.rs    -- Command bar input widget and status display
```

### Data Flow

```
AppState (shared services)
    |
    v
App::refresh()              <-- called on startup and every tick
    |
    +-- agent_store.list_agents()
    +-- agent_orchestrator.list_teams()
    +-- approval_manager.list_approvals()
    +-- capacity_broker.list_sources() / get_total_cost()
    +-- analytics.get_recent_traces()
    |
    v
DashboardData (cached)      <-- consumed by all view renderers
    |
    v
render_*() functions         <-- pure: DashboardData -> Frame
```

### Key Design Decisions

1. **Same service layer as REPL and HTTP API.** The TUI constructs `AppState` identically to the other modes. No data duplication or separate backend.

2. **Tick-based refresh.** An async event handler multiplexes crossterm key events with periodic ticks (default 2s). Each tick re-queries all services.

3. **Command bar reuses `cli::tools::execute_tool()`.** The same tool functions the Chief of Staff REPL uses are available via slash commands.

4. **Panic-safe terminal restore.** A custom panic hook ensures the terminal is restored to normal mode even if the app crashes.

5. **ratatui with crossterm backend.** ratatui 0.29 uses crossterm 0.28, which matches the existing workspace dependency exactly.

### Dependencies

| Crate | Purpose |
|---|---|
| `ratatui` 0.29 | Terminal UI framework (widgets, layout, rendering) |
| `crossterm` 0.28 | Terminal manipulation (raw mode, events, alternate screen) |
| `tokio` | Async runtime for event loop and data refresh |

---

## Testing

### Running Tests

```bash
export SHIIOO_ENCRYPTION_KEY=$(openssl rand -base64 32 | head -c 32)
cargo test --package shiioo-server
```

### Test Coverage

**53 TUI-specific tests** organized in two modules:

#### Unit Tests (`tui::app::tests`) -- 29 tests

Test pure functions and data structures without requiring `AppState`:

- **`shell_split`** (6 tests) -- argument parsing with quotes, spaces, edge cases
- **`parse_command`** (17 tests) -- all 7 commands with aliases, filters, error cases
- **View/data types** (6 tests) -- equality, selection wrap-around, employee lookup

#### Integration Tests (`tui::tests::integration`) -- 24 tests

Test the full `App` with a real `AppState` backed by temp directories:

- **App lifecycle** (3 tests) -- initial state, refresh empty, refresh with employees
- **Navigation** (6 tests) -- selection, employee detail, view transitions, empty list
- **Command execution** (5 tests) -- `/status`, `/hire` with auto-refresh, unknown, empty
- **Command editing** (3 tests) -- enter/exit mode, insert/delete, cursor movement
- **Rendering smoke tests** (7 tests) -- every view renders without panic, content verified via `TestBackend` buffer introspection

Integration tests use `ratatui::backend::TestBackend` to render views into an in-memory buffer and assert on content without requiring a real terminal.
