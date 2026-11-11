//! The vocabulary and the rules of the product. Pure: no I/O, no clock,
//! no serialization. Everything here can be tested with plain values.

pub mod board;
pub mod change_request;
pub mod desk;
pub mod filter;
pub mod item;
pub mod rules;
pub mod task;
pub mod ticket;
pub mod time;
pub mod triage;
pub mod urgency;

#[cfg(test)]
mod triage_tests;
