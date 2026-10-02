// https://github.com/monji024

use std::collections::BTreeMap;
use std::io::{self, BufRead};
use std::path::PathBuf;

use crate::state::{StackFrame, VarKind};

const INTERNAL_VARS: &[&str] = &[
    "BASH", "BASHOPTS", "BASHPID", "BASH_ALIASES", "BASH_ARGC", "BASH_ARGV",
    "BASH_ARGV0", "BASH_CMDS", "BASH_COMMAND", "BASH_LINENO", "BASH_REMATCH",
    "BASH_SOURCE", "BASH_SUBSHELL", "BASH_VERSINFO", "BASH_VERSION", "COLUMNS",
    "COMP_WORDBREAKS", "DIRSTACK", "EPOCHREALTIME", "EPOCHSECONDS", "EUID",
    "FUNCNAME", "GROUPS", "HISTCMD", "HISTFILE", "HISTFILESIZE", "HISTSIZE",
    "HOSTNAME", "HOSTTYPE", "IFS", "LINES", "MACHTYPE", "MAILCHECK", "OPTERR",
    "OPTIND", "OSTYPE", "PATH", "PIPESTATUS", "PPID", "PS1", "PS2", "PS4",
    "PWD", "RANDOM", "SECONDS", "SHELL", "SHELLOPTS", "SHLVL", "SRANDOM",
    "TERM", "UID", "_", "LS_COLORS", "XDG_DATA_DIRS", "XDG_CONFIG_DIRS",
    "DBUS_SESSION_BUS_ADDRESS",
];

#[derive(Debug, Clone)]
pub struct RawFrame {
    pub is_sync: bool,
    pub file: PathBuf,
    pub line: usize,
    pub exit_code: i32,
    pub subshell: usize,
    pub pipe_status: Vec<i32>,
    pub pwd: String,
    pub cmd: String,
    pub stack: Vec<StackFrame>,
    pub vars: BTreeMap<String, (VarKind, String)>,
}

pub fn read_lossy_line<R: BufRead>(reader: &mut R, buf: &mut Vec<u8>) -> io::Result<Option<String>> {
    buf.clear();
    let n = reader.read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    }
    Ok(Some(String::from_utf8_lossy(buf).into_owned()))
}

pub fn parse_frame<R: BufRead>(
    header: &str,
    reader: &mut R,
    nonce: &str,
) -> io::Result<Option<RawFrame>> {
    let mut parts = header.splitn(6, '\t');
    let is_sync = match parts.next() {
        Some("STOP") => false,
        Some("SYNC") => true,
        _ => return Ok(None),
