//! aki: whose turn is it?
//!
//! Dependencies point inward only:
//! `app` (composition) → `adapters` (details) → `use_cases` (policy) → `entities` (rules).

mod adapters;
mod app;
mod entities;
mod use_cases;

use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    app::run(&arguments)
}
