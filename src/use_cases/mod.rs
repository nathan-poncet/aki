//! Application policy: what to read, in which order, and what to do with it.

pub mod check_sources;
pub mod collect_board;
pub mod launch_agent;
pub mod open_desk;
pub mod ports;
pub mod set_aside;
pub mod watch_board;

#[cfg(test)]
pub mod testing;
