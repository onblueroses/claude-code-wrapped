use crate::readers::{
    compatibility_ingest, emit_compatibility_coverage, emit_compatibility_error, IngestionReadError,
};
use crate::{DataCoverage, SessionBreakdown};
use std::path::Path;

/// Reads the session tree through the bounded, privacy-safe normalized pipeline.
///
/// This preserves the original infallible signature. Prefer
/// [`try_read_session_breakdown`] when coverage or an actionable ingestion error is required.
pub fn read_session_breakdown(projects_dir: &Path, year: Option<i32>) -> SessionBreakdown {
    match try_read_session_breakdown(projects_dir, year) {
        Ok((breakdown, coverage)) => {
            emit_compatibility_coverage(&coverage, "read_session_breakdown");
            breakdown
        }
        Err(error) => {
            emit_compatibility_error(&error, "read_session_breakdown");
            SessionBreakdown::default()
        }
    }
}

/// Returns a privacy-safe session tree together with ingestion coverage.
pub fn try_read_session_breakdown(
    projects_dir: &Path,
    year: Option<i32>,
) -> Result<(SessionBreakdown, DataCoverage), IngestionReadError> {
    let ingested = compatibility_ingest(projects_dir, year)?;
    Ok((ingested.session_breakdown, ingested.coverage))
}
