// https://github.com/monji024

const SENSITIVE_NAME_PATTERNS: &[&str] = &[
    "PASS",
    "PASSWORD",
    "PASSWD",
    "SECRET",
    "TOKEN",
    "API_KEY",
    "APIKEY",
    "PRIVATE_KEY",
    "AUTH",
    "CREDENTIAL",
    "CREDENTIALS",
    "BEARER",
    "SESSION_KEY",
    "ACCESS_KEY",
    "AWS_SECRET",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "NPM_TOKEN",
    "DATABASE_URL",
    "DB_PASS",
    "COOKIE",
];

pub fn is_sensitive_var_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    for pat in SENSITIVE_NAME_PATTERNS {
        if upper.contains(pat) {
            return true;
        }
    }
    false
}

pub fn looks_like_secret_value(val: &str) -> bool {
    let trimmed = val.trim();
    trimmed.starts_with("ghp_")
        || trimmed.starts_with("gho_")
        || trimmed.starts_with("github_pat_")
        || trimmed.starts_with("sk-")
        || trimmed.starts_with("AKIA")
        || trimmed.starts_with("eyJ")
        || trimmed.contains("-----BEGIN ")
        || trimmed.contains("Bearer ")
}

pub fn redact_value(name: &str, value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    if is_sensitive_var_name(name) || looks_like_secret_value(value) {
        let char_count = value.chars().count();
        if char_count <= 4 {
            return "******** [REDACTED]".to_string();
        }
        let prefix: String = value.chars().take(2).collect();
        return format!("{}****** ({} chars) [REDACTED]", prefix, char_count);
    }
    value.to_string()
}

pub fn redact_command_args(cmd: &str) -> String {
    let mut out = cmd.to_string();
    for flag in &[
        "--password=",
        "--token=",
        "--api-key=",
        "--secret=",
        "Authorization: Bearer ",
    ] {
        if let Some(pos) = out.find(flag) {
            let val_start = pos + flag.len();
            let rest = &out[val_start..];
            let end_offset = rest
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .unwrap_or(rest.len());
            out.replace_range(val_start..(val_start + end_offset), "[REDACTED]");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_and_redacts_secrets() {
        assert!(is_sensitive_var_name("DB_PASSWORD"));
        assert!(is_sensitive_var_name("aws_secret_access_key"));
        assert!(is_sensitive_var_name("GITHUB_TOKEN"));
        assert!(!is_sensitive_var_name("TARGET_HOST"));

        let red = redact_value("DB_PASSWORD", "supersecret123");
        assert!(red.contains("[REDACTED]"));
        assert!(!red.contains("supersecret123"));

        let gh = redact_value("NORMAL_VAR", "ghp_1234567890abcdef");
        assert!(gh.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_command_line_tokens() {
        let cmd = "curl -H \"Authorization: Bearer my_secret_jwt\" --password=topsecret https://example.com";
        let clean = redact_command_args(cmd);
        assert!(!clean.contains("my_secret_jwt"));
        assert!(!clean.contains("topsecret"));
        assert!(clean.contains("[REDACTED]"));
    }
}
