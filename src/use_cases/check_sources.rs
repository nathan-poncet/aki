//! Asks every source who I am: the quickest way to see what is connected.

use std::thread;

use super::ports::{SourceError, WorkSource};

pub struct SourceCheck {
    pub source: String,
    /// The identity the source reports, or why it could not.
    pub outcome: Result<String, SourceError>,
}

pub fn check_sources(sources: &[Box<dyn WorkSource>]) -> Vec<SourceCheck> {
    thread::scope(|scope| {
        let checks: Vec<_> = sources
            .iter()
            .map(|source| scope.spawn(move || source.identity()))
            .collect();
        sources
            .iter()
            .zip(checks)
            .map(|(source, check)| SourceCheck {
                source: source.name().to_string(),
                outcome: check.join().expect("a source panicked"),
            })
            .collect()
    })
}
