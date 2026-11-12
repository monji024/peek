// https://github.com/monji024

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::time::Duration;

use super::{StackFrame, VarEntry};

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub step_index: usize,
    pub file: PathBuf,
    pub line: usize,
    pub exit_code: i32,
    pub subshell: usize,
    pub pipe_status: Vec<i32>,
    pub pwd: String,
    pub cmd: String,
    pub stack: Vec<StackFrame>,
    pub vars: BTreeMap<String, VarEntry>,
    pub latency: Duration,
}

pub struct HistoryBuffer {
    frames: VecDeque<Snapshot>,
    cursor: Option<usize>,
    capacity: usize,
}

impl HistoryBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            frames: VecDeque::with_capacity(capacity),
            cursor: None,
            capacity,
        }
    }

    pub fn push(&mut self, snapshot: Snapshot) {
        if self.frames.len() >= self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(snapshot);
        self.cursor = None;
    }

    pub fn clear(&mut self) {
        self.frames.clear();
        self.cursor = None;
    }

    pub fn is_time_traveling(&self) -> bool {
        self.cursor.is_some()
    }

    pub fn active(&self) -> Option<&Snapshot> {
        match self.cursor {
            Some(idx) => self.frames.get(idx),
            None => self.frames.back(),
        }
    }

    pub fn step_back(&mut self) -> bool {
        if self.frames.len() < 2 {
            return false;
        }
        match self.cursor {
            None => {
                self.cursor = Some(self.frames.len() - 2);
                true
            }
            Some(idx) if idx > 0 => {
                self.cursor = Some(idx - 1);
                true
            }
            _ => false,
        }
    }

    pub fn step_forward(&mut self) -> bool {
        match self.cursor {
            Some(idx) if idx + 1 < self.frames.len() - 1 => {
                self.cursor = Some(idx + 1);
                true
            }
            Some(_) => {
                self.cursor = None;
                true
            }
            None => false,
        }
    }

    pub fn return_to_live(&mut self) {
        self.cursor = None;
    }

    pub fn position_badge(&self) -> Option<(usize, usize)> {
        let total = self.frames.len();
        if total == 0 {
            return None;
        }
        let pos = self.cursor.map(|i| i + 1).unwrap_or(total);
        Some((pos, total))
    }
}