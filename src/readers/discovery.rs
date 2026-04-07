use std::fs;
use std::path::{Path, PathBuf};

pub fn discover_jsonl_files(projects_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_jsonl_files(projects_dir, &mut files);
    files
}

pub fn discover_session_files(projects_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for project_dir in read_dir_sorted(projects_dir)
        .into_iter()
        .filter(|path| path.is_dir())
    {
        for file in read_dir_sorted(&project_dir)
            .into_iter()
            .filter(|path| path.is_file())
            .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        {
            files.push(file);
        }
    }
    files
}

fn collect_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for path in read_dir_sorted(dir) {
        if path.is_dir() {
            collect_jsonl_files(&path, files);
        } else if path.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
            files.push(path);
        }
    }
}

fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}
