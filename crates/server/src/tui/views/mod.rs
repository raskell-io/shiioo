mod command;
mod dashboard;
mod employee;
mod logs;
mod teams;

pub use command::render_command_area;
pub use dashboard::render_dashboard;
pub use employee::render_employee_detail;
pub use logs::render_logs;
pub use teams::render_teams;
