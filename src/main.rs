// https://github.com/monji024

mod app;
mod engine;
mod keymap;
mod state;
mod ui;

use std::fs::OpenOptions;
use std::io;
use std::os::unix::io::AsRawFd;
use std::panic;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::{App, PromptMode, TtyAction};
use keymap::normalize_command_key;

