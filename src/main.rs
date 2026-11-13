// https://github.com/monji024

mod app;
mod engine;
mod keymap;
mod protocol;
mod security;
mod session_mgr;
mod snapshot;
mod state;
mod ui;

use std::fs::OpenOptions;
use std::io;
use std::os::unix::io::AsRawFd;
use std::panic;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Parser;
use crossterm::{
    cursor,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::{App, PromptMode, TtyAction};
use keymap::normalize_command_key;
use session_mgr::{
    cleanup_stale_sessions, list_active_sessions, resolve_session_socket, run_headless_server,
    RemoteClient,
};
use ui::theme::ThemeMode;

#[derive(Parser, Debug)]
#[command(
    name = "peek",
    author = "monji024 (https://github.com/monji024)",
    version = "1.1.0",
    about = "Interactive TUI debugger, profiler & time-travel inspector for Bash scripts",
    after_help = "https://github.com/monji024/peek"
)]
struct Cli {
    #[arg(help = "Target Bash script to debug")]
    script: Option<PathBuf>,

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

    #[arg(
        short = 'L',
        long = "listen",
        num_args = 0..=1,
        default_missing_value = "",
        help = "Run target script in this terminal and listen for a separate-terminal TUI client"
    )]
    listen: Option<String>,

    #[arg(
        short = 'a',
        long = "attach",
        num_args = 0..=1,
        default_missing_value = "latest",
        help = "Attach TUI client to a running debugging session (by session ID, PID, or 'latest')"
    )]
    attach: Option<String>,

    #[arg(
        long = "list-sessions",
        help = "List active local debugging sessions and clean up stale sockets"
    )]
    list_sessions: bool,

    #[arg(
        long = "theme",
        default_value = "classic",
        help = "Visual theme mode: classic, luna-blue, or terminal-native"
    )]
    theme: String,

    #[arg(
        long = "snapshot-dir",
        help = "Optional directory path to scope filesystem snapshots and diffs"
    )]
    snapshot_dir: Option<PathBuf>,
}

fn main() -> Result<()> {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        libc::signal(libc::SIGTTOU, libc::SIG_IGN);
        libc::signal(libc::SIGTTIN, libc::SIG_IGN);
    }

    let cli = Cli::parse();

    if cli.list_sessions {
        let removed = cleanup_stale_sessions().unwrap_or(0);
        let sessions = list_active_sessions()?;
        if sessions.is_empty() {
            println!("No active peek debugging sessions ({} stale cleaned).", removed);
        } else {
            println!("SESSION ID           PID      SCRIPT");
            for s in sessions {
                println!("{:<20} {:<8} {}", s.session_id, s.root_pid, s.entry_script);
            }
        }
        return Ok(());
    }

    let theme_mode = ThemeMode::from_str(&cli.theme);

    if let Some(target_session) = cli.attach {
        let sock_path = resolve_session_socket(&target_session)?;
        let client = RemoteClient::connect(&sock_path)?;
        let mut app = App::new_attached(
            client,
            &cli.break_lines,
            cli.break_on_error,
            theme_mode,
            cli.snapshot_dir,
        )?;
        return run_tui(&mut app);
    }

    let Some(script_path) = cli.script else {
        bail!("missing <SCRIPT> argument (or pass --attach / --list-sessions; see --help)");
    };

    let entry_script = std::fs::canonicalize(&script_path)
        .with_context(|| format!("failed to resolve script path: {:?}", script_path))?;

    if let Some(listen_name) = cli.listen {
        let opt_name = if listen_name.trim().is_empty() {
            None
        } else {
            Some(listen_name)
        };
        let exit_code = run_headless_server(
            opt_name,
            entry_script,
            cli.args,
            &cli.break_lines,
            cli.break_on_error,
        )?;
        std::process::exit(exit_code);
    }

    let mut app = App::new(
        entry_script,
        cli.args,
        &cli.break_lines,
        cli.break_on_error,
        theme_mode,
        cli.snapshot_dir,
    )?;

    run_tui(&mut app)
}

fn run_tui(app: &mut App) -> Result<()> {
    install_panic_hook();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, cursor::Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_event_loop(&mut terminal, app);

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
    let _ = execute!(
        io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        cursor::Show
    );
}

fn resume_tui(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    reclaim_foreground_tty();
    enable_raw_mode()?;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        cursor::Hide
    )?;
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
                app.send_tty_granted()?;
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
                        app.backend.shutdown();
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
                Event::Resize(_, _) => {
                    terminal.autoresize()?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}