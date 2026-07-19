extern crate self as ccwrapped;

pub mod analyzers;
pub mod fmt;
mod ingestion;
pub mod readers;
pub mod renderers;
pub mod report;
#[cfg(windows)]
mod windows_private_acl;
pub use fmt::*;
pub use report::*;

pub(crate) const PARTIAL_USAGE_LIMITATION: &str =
    "Usage evidence is partial; values are limited to observed source coverage.";

pub(crate) fn analytical_capability_available(report: &Report, capability: &str) -> bool {
    match report
        .data_coverage
        .capabilities
        .get(capability)
        .map(String::as_str)
    {
        Some("available") => true,
        Some(_) => false,
        None => !report
            .data_coverage
            .capabilities
            .contains_key("analysis_usage_totals"),
    }
}
