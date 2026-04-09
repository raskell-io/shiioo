mod dashboard;
mod employee;
mod logs;
mod teams;

pub use dashboard::render_dashboard;
pub use employee::render_employee_detail;
pub use logs::render_logs;
pub use teams::render_teams;
