use super::{compatibility_discover, emit_compatibility_error, IngestionReadError};
use crate::ingestion::CompatibilityPathScope;
use std::path::{Path, PathBuf};

pub fn discover_jsonl_files(projects_dir: &Path) -> Vec<PathBuf> {
    match try_discover_jsonl_files(projects_dir) {
        Ok(files) => files,
        Err(error) => {
            emit_compatibility_error(&error, "discover_jsonl_files");
            Vec::new()
        }
    }
}

pub fn discover_session_files(projects_dir: &Path) -> Vec<PathBuf> {
    match try_discover_session_files(projects_dir) {
        Ok(files) => files,
        Err(error) => {
            emit_compatibility_error(&error, "discover_session_files");
            Vec::new()
        }
    }
}

/// Discovers JSONL paths through the bounded, root-confined transcript adapter.
pub fn try_discover_jsonl_files(projects_dir: &Path) -> Result<Vec<PathBuf>, IngestionReadError> {
    compatibility_discover(projects_dir, CompatibilityPathScope::AllJsonl)
}

/// Discovers direct `projects/<project>/<session>.jsonl` paths through the bounded adapter.
pub fn try_discover_session_files(projects_dir: &Path) -> Result<Vec<PathBuf>, IngestionReadError> {
    compatibility_discover(projects_dir, CompatibilityPathScope::DirectSessions)
}
