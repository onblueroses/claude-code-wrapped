use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct PatternError {
    message: String,
}

impl Display for PatternError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for PatternError {}

#[derive(Debug, Clone)]
pub struct GlobError {
    path: PathBuf,
    message: String,
}

impl GlobError {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Display for GlobError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.path.display(), self.message)
    }
}

impl Error for GlobError {}

pub struct Paths {
    entries: Vec<Result<PathBuf, GlobError>>,
    index: usize,
}

impl Iterator for Paths {
    type Item = Result<PathBuf, GlobError>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.entries.get(self.index).cloned();
        self.index += 1;
        item
    }
}

pub fn glob(pattern: &str) -> Result<Paths, PatternError> {
    let pattern = pattern.replace('\\', "/");
    let (root, mode) = if let Some(root) = pattern.strip_suffix("/**/*.jsonl") {
        (PathBuf::from(root), SearchMode::RecursiveJsonl)
    } else if let Some(root) = pattern.strip_suffix("/*/*.jsonl") {
        (PathBuf::from(root), SearchMode::OneLevelJsonl)
    } else {
        return Err(PatternError {
            message: format!("unsupported pattern: {pattern}"),
        });
    };

    let mut entries = Vec::new();
    match mode {
        SearchMode::RecursiveJsonl => collect_recursive(&root, &mut entries),
        SearchMode::OneLevelJsonl => collect_one_level(&root, &mut entries),
    }
    entries.sort_by(|left, right| {
        path_of(left)
            .display()
            .to_string()
            .cmp(&path_of(right).display().to_string())
    });

    Ok(Paths { entries, index: 0 })
}

#[derive(Clone, Copy)]
enum SearchMode {
    RecursiveJsonl,
    OneLevelJsonl,
}

fn collect_recursive(root: &Path, entries: &mut Vec<Result<PathBuf, GlobError>>) {
    let Ok(read_dir) = fs::read_dir(root) else {
        return;
    };

    for entry in read_dir {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if path.is_dir() {
                    collect_recursive(&path, entries);
                } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                    entries.push(Ok(path));
                }
            }
            Err(error) => entries.push(Err(GlobError {
                path: root.to_path_buf(),
                message: error.to_string(),
            })),
        }
    }
}

fn collect_one_level(root: &Path, entries: &mut Vec<Result<PathBuf, GlobError>>) {
    let Ok(read_dir) = fs::read_dir(root) else {
        return;
    };

    for entry in read_dir {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let Ok(children) = fs::read_dir(&path) else {
                    continue;
                };
                for child in children {
                    match child {
                        Ok(child) => {
                            let file_path = child.path();
                            if file_path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                                entries.push(Ok(file_path));
                            }
                        }
                        Err(error) => entries.push(Err(GlobError {
                            path: path.clone(),
                            message: error.to_string(),
                        })),
                    }
                }
            }
            Err(error) => entries.push(Err(GlobError {
                path: root.to_path_buf(),
                message: error.to_string(),
            })),
        }
    }
}

fn path_of(result: &Result<PathBuf, GlobError>) -> PathBuf {
    match result {
        Ok(path) => path.clone(),
        Err(error) => error.path.clone(),
    }
}
