// https://github.com/monji024

pub mod history;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    Scalar,
    Exported,
    ReadOnly,
    Integer,
    Array,
    AssocArray,
}

impl VarKind {
    pub fn tag(self) -> &'static str {
        match self {
            VarKind::Scalar => "var",
            VarKind::Exported => "env",
            VarKind::ReadOnly => "ro ",
            VarKind::Integer => "int",
            VarKind::Array => "arr",
            VarKind::AssocArray => "map",
        }
    }
}

#[derive(Debug, Clone)]
pub struct VarEntry {
    pub name: String,
    pub value: String,
    pub prev_value: Option<String>,
    pub kind: VarKind,
    pub changed: bool,
}

#[derive(Debug, Clone)]
pub struct StackFrame {
    pub func: String,
    pub file: PathBuf,
    pub line: usize,
}

#[derive(Debug, Clone, Default)]
pub struct LineProfile {
    pub hits: usize,
    pub total_us: u128,
}

#[derive(Debug, Clone)]
pub enum LogStream {
    Stdout,
    Stderr,
    Eval,
    Stdin,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub stream: LogStream,
    pub text: String,
}

pub struct BoundedLog {
    entries: VecDeque<LogEntry>,
    max_lines: usize,
}

impl BoundedLog {
    pub fn new(max_lines: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(1024),
            max_lines,
        }
    }

    pub fn push(&mut self, stream: LogStream, text: String) {
        if self.entries.len() >= self.max_lines {
            self.entries.pop_front();
        }
        self.entries.push_back(LogEntry { stream, text });
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &LogEntry> {
        self.entries.iter()
    }
}

pub struct SourceCache {
    files: HashMap<PathBuf, Vec<String>>,
    profiles: HashMap<(PathBuf, usize), LineProfile>,
}

impl SourceCache {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            profiles: HashMap::new(),
        }
    }

    pub fn ensure_loaded(&mut self, path: &Path) -> &[String] {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.files.entry(canonical).or_insert_with(|| match std::fs::read(path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes)
                .lines()
                .map(String::from)
                .collect(),
            Err(e) => vec![format!("# [peek: unable to read {:?}: {}]", path, e)],
        })
    }

    pub fn get_lines(&self, path: &Path) -> &[String] {
        self.files.get(path).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn record_profile(&mut self, file: PathBuf, line: usize, elapsed: Duration) {
        let entry = self.profiles.entry((file, line)).or_default();
        entry.hits += 1;
        entry.total_us += elapsed.as_micros();
    }

    pub fn get_profile(&self, file: &Path, line: usize) -> Option<&LineProfile> {
        self.profiles.get(&(file.to_path_buf(), line))
    }

    pub fn clear_profiles(&mut self) {
        self.profiles.clear();
    }
}