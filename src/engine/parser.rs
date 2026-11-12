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
    };

    let file = PathBuf::from(parts.next().unwrap_or(""));
    let line = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let exit_code = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let subshell = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let pipe_status = parts
        .next()
        .unwrap_or("")
        .split_whitespace()
        .filter_map(|s| s.parse::<i32>().ok())
        .collect();

    let vars_begin_marker = format!("__PEEK_VARS_{}__", nonce);
    let frame_end_marker = format!("__PEEK_END_{}__", nonce);

    let mut pwd = String::new();
    let mut cmd = String::new();
    let mut stack = Vec::new();
    let mut raw_var_lines = Vec::new();
    let mut in_vars = false;
    let mut raw_buf = Vec::with_capacity(512);

    while let Some(row) = read_lossy_line(reader, &mut raw_buf)? {
        if row == frame_end_marker {
            break;
        }
        if in_vars {
            raw_var_lines.push(row);
            continue;
        }
        if row == vars_begin_marker {
            in_vars = true;
        } else if let Some(p) = row.strip_prefix("PWD:") {
            pwd = p.to_string();
        } else if let Some(c) = row.strip_prefix("CMD:") {
            cmd = c.to_string();
        } else if let Some(f) = row.strip_prefix("FRAME:") {
            let mut fp = f.splitn(3, '\t');
            let func = fp.next().unwrap_or("?").to_string();
            let fpath = PathBuf::from(fp.next().unwrap_or("?"));
            let fline = fp.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            stack.push(StackFrame {
                func,
                file: fpath,
                line: fline,
            });
        }
    }

    let vars = parse_declare_block(&raw_var_lines);

    Ok(Some(RawFrame {
        is_sync,
        file,
        line,
        exit_code,
        subshell,
        pipe_status,
        pwd,
        cmd,
        stack,
        vars,
    }))
}

pub fn parse_declare_block(lines: &[String]) -> BTreeMap<String, (VarKind, String)> {
    let mut logical_entries: Vec<String> = Vec::new();

    for line in lines {
        if line.starts_with("declare -") {
            logical_entries.push(line.clone());
        } else if let Some(last) = logical_entries.last_mut() {
            if !line.is_empty() {
                last.push('\n');
                last.push_str(line);
            }
        }
    }

    let mut map = BTreeMap::new();
    for entry in logical_entries {
        if let Some((name, kind, val)) = parse_single_declare(&entry) {
            if !is_ignored_var(&name) {
                map.insert(name, (kind, val));
            }
        }
    }
    map
}
fn is_ignored_var(name: &str) -> bool {
    name == "LINENO"
        || name == "FUNCNAME"
        || name == "GROUPS"
        || name == "DIRSTACK"
        || name.starts_with("BASH_")
        || name.starts_with("__peek_")
        || name.starts_with("__PEEK_")
        || name.starts_with("PEEK_")
        || name.starts_with("__")
        || INTERNAL_VARS.contains(&name)
}

fn parse_single_declare(line: &str) -> Option<(String, VarKind, String)> {
    let rest = line.strip_prefix("declare ")?;
    let mut parts = rest.splitn(2, ' ');
    let flags = parts.next()?;
    let body = parts.next()?;

    let kind = if flags.contains('A') {
        VarKind::AssocArray
    } else if flags.contains('a') {
        VarKind::Array
    } else if flags.contains('r') {
        VarKind::ReadOnly
    } else if flags.contains('x') {
        VarKind::Exported
    } else if flags.contains('i') {
        VarKind::Integer
    } else {
        VarKind::Scalar
    };

    let (name, raw_val) = match body.split_once('=') {
        Some((n, v)) => (n, v),
        None => (body, ""),
    };

    if name.is_empty() || name.contains(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        return None;
    }

    let clean = unquote_bash_value(raw_val);
    Some((name.to_string(), kind, clean))
}

fn unquote_bash_value(raw: &str) -> String {
    if let Some(inner) = raw.strip_prefix("$'").and_then(|s| s.strip_suffix('\'')) {
        return inner.to_string();
    }
    if let Some(inner) = raw.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return inner
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
            .replace("\\$", "$")
            .replace("\\`", "`");
    }
    raw.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_declare_output_cleanly() {
        let dump = vec![
            r#"declare -- TARGET_HOST="10.0.0.1""#.to_string(),
            r#"declare -i PORT="8080""#.to_string(),
            r#"declare -a SERVERS=([0]="alpha" [1]="beta")"#.to_string(),
            r#"declare -- BODY="{"#.to_string(),
            r#"  \"ok\": true"#.to_string(),
            r#"}""#.to_string(),
            r#"declare -- __PEEK_NONCE="deadbeef""#.to_string(),
        ];

        let parsed = parse_declare_block(&dump);
        assert_eq!(parsed.get("TARGET_HOST"), Some(&(VarKind::Scalar, "10.0.0.1".into())));
        assert_eq!(parsed.get("PORT"), Some(&(VarKind::Integer, "8080".into())));
        assert_eq!(parsed.get("SERVERS").map(|x| &x.0), Some(&VarKind::Array));
        assert_eq!(
            parsed.get("BODY"),
            Some(&(VarKind::Scalar, "{\n  \"ok\": true\n}".into()))
        );
        assert!(!parsed.contains_key("__PEEK_NONCE"));
    }
}