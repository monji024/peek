// https://github.com/monji024

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    High,
    Warning,
    Advisory,
}

impl DiagnosticSeverity {
    pub fn tag(&self) -> &'static str {
        match self {
            DiagnosticSeverity::High => "HIGH",
            DiagnosticSeverity::Warning => "WARN",
            DiagnosticSeverity::Advisory => "INFO",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScriptDiagnostic {
    pub line: usize,
    pub code: &'static str,
    pub severity: DiagnosticSeverity,
    pub message: String,
}

pub fn analyze_bash_source(lines: &[String]) -> Vec<ScriptDiagnostic> {
    let mut out = Vec::new();
    let mut has_strict_mode = false;

    for (idx, raw) in lines.iter().enumerate() {
        let lno = idx + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let code_part = strip_inline_comment(trimmed);

        if code_part.contains("set -e")
            || code_part.contains("set -eu")
            || code_part.contains("errexit")
        {
            has_strict_mode = true;
        }

        if code_part.contains("rm -rf /")
            || code_part.contains("rm -rf \"$/")
            || code_part.contains("rm -rf $")
        {
            out.push(ScriptDiagnostic {
                line: lno,
                code: "PK001",
                severity: DiagnosticSeverity::High,
                message: "Unquoted or root-targeted `rm -rf` variable expansion; use `${VAR:?}` guard".into(),
            });
        }

        if code_part.starts_with("eval ") || code_part.contains("; eval ") || code_part.contains("&& eval ") {
            out.push(ScriptDiagnostic {
                line: lno,
                code: "PK002",
                severity: DiagnosticSeverity::High,
                message: "Dynamic `eval` statement can execute arbitrary injected shell tokens".into(),
            });
        }

        if code_part.starts_with("cd ")
            && !code_part.contains("||")
            && !code_part.contains("&&")
            && !has_strict_mode
        {
            out.push(ScriptDiagnostic {
                line: lno,
                code: "PK003",
                severity: DiagnosticSeverity::Warning,
                message: "Unchecked `cd` without `|| return` or `set -e` may continue in wrong directory".into(),
            });
        }

        if has_unquoted_backticks(code_part) {
            out.push(ScriptDiagnostic {
                line: lno,
                code: "PK004",
                severity: DiagnosticSeverity::Advisory,
                message: "Legacy backtick `` `...` `` command substitution; prefer `$(...)` for nesting clarity".into(),
            });
        }

        if (code_part.starts_with("local ") || code_part.starts_with("export ") || code_part.starts_with("readonly "))
            && code_part.contains("$(")
        {
            out.push(ScriptDiagnostic {
                line: lno,
                code: "PK005",
                severity: DiagnosticSeverity::Warning,
                message: "Declaration with inline `$(...)` masks the subshell exit status (`$?` becomes 0)".into(),
            });
        }

        if (code_part.starts_with("read ") || code_part == "read") && !code_part.contains("-r") {
            out.push(ScriptDiagnostic {
                line: lno,
                code: "PK006",
                severity: DiagnosticSeverity::Advisory,
                message: "`read` without `-r` mangles backslashes in input lines".into(),
            });
        }
    }

    out
}

fn strip_inline_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && in_double {
            i += 2;
            continue;
        }
        if b == b'\'' && !in_double {
            in_single = !in_single;
        } else if b == b'"' && !in_single {
            in_double = !in_double;
        } else if b == b'#' && !in_single && !in_double && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return &line[..i];
        }
        i += 1;
    }
    line
}

fn has_unquoted_backticks(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut in_single = false;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            in_single = !in_single;
        } else if bytes[i] == b'`' && !in_single {
            return true;
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_bash_pitfalls() {
        let script = vec![
            "#!/usr/bin/env bash".to_string(),
            "cd /some/missing/dir".to_string(),
            "local output=$(false)".to_string(),
            "read user_line".to_string(),
            "eval \"$UNTRUSTED\"".to_string(),
        ];
        let diags = analyze_bash_source(&script);
        assert!(diags.iter().any(|d| d.code == "PK003" && d.line == 2));
        assert!(diags.iter().any(|d| d.code == "PK005" && d.line == 3));
        assert!(diags.iter().any(|d| d.code == "PK006" && d.line == 4));
        assert!(diags.iter().any(|d| d.code == "PK002" && d.line == 5));
    }
}
