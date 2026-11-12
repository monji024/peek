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

#[derive(Parser, Debug)]
#[command(
    name = "peek",
    author = "monji024 (https://github.com/monji024)",
    version = "1.0.0",
    about = "Interactive TUI debugger, profiler & time-travel inspector for Bash scripts",
    after_help = "https://github.com/monji024/peek"
)]
struct Cli {
    #[arg(required = true, help = "Target Bash script to debug")]
    script: PathBuf,

    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        help = "Arguments forwarded to the target script ($@)"
    )]
    args: Vec<String>,

    #[arg(
        short,
        long,
        value_delimiter = ',',
        help = "Initial line breakpoints in the entry script (e.g. -b 10,24)"
    )]
    break_lines: Vec<usize>,

    #[arg(
        short = 'e',
        long,
        help = "Automatically pause whenever any command exits with non-zero status ($? != 0)"
    )]
    break_on_error: bool,
}

fn main() -> Result<()> {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        libc::signal(libc::SIGTTOU, libc::SIG_IGN);
        libc::signal(libc::SIGTTIN, libc::SIG_IGN);
    }

    let cli = Cli::parse();
    let entry_script = std::fs::canonicalize(&cli.script)
        .with_context(|| format!("failed to resolve script path: {:?}", cli.script))?;

    let mut app = App::new(
        entry_script,
        cli.args,
        &cli.break_lines,
        cli.break_on_error,
    )?;

    install_panic_hook();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_event_loop(&mut terminal, &mut app);

    reclaim_foreground_tty();
    restore_terminal();
    res
}

fn install_panic_hook() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        reclaim_foreground_tty();
        restore_terminal();
        default_hook(info);
    }));
}

fn handoff_foreground_tty(child_pgid: i32) {
    if let Ok(tty) = OpenOptions::new().read(true).write(true).open("/dev/tty") {
        unsafe {
            libc::tcsetpgrp(tty.as_raw_fd(), child_pgid);
        }
    }
}

fn reclaim_foreground_tty() {
    if let Ok(tty) = OpenOptions::new().read(true).write(true).open("/dev/tty") {
        unsafe {
            let pgrp = libc::getpgrp();
            libc::tcsetpgrp(tty.as_raw_fd(), pgrp);
        }
    }
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

fn resume_tui(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    reclaim_foreground_tty();
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    terminal.clear()?;
    Ok(())
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        match app.tick()? {
            TtyAction::Acquire(cmd) => {
                restore_terminal();
                handoff_foreground_tty(app.pid() as i32);
                app.session.send_tty_granted()?;
                app.status = format!("Completed interactive TTY handoff: {}", cmd);
                continue;
            }
            TtyAction::Release => {
                resume_tui(terminal)?;
            }
            TtyAction::None => {}
        }

        if app.tty_passthrough {
            thread::sleep(Duration::from_millis(16));
            continue;
        }

        terminal.draw(|f| ui::draw(f, app))?;

        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('ز'))
                    {
                        app.session.shutdown();
                        break;
                    }

                    let code = if app.prompt_mode == PromptMode::None {
                        normalize_command_key(key)
                    } else {
                        key.code
                    };

                    if app.handle_key(code)? {
                        break;
                    }
                }
                Event::Mouse(mouse_ev) => {
                    app.handle_mouse(mouse_ev);
                }
                _ => {}
            }
        }
    }
    Ok(())
}