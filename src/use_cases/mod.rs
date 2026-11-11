//! Application policy: what to read, in which order, and what to do with it.

pub mod check_sources;
pub mod collect_board;
pub mod open_desk;
pub mod ports;
pub mod watch_board;

#[cfg(test)]
pub mod testing;
