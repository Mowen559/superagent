//! Agent command-line argument processing and filtering.
//!
//! Mirrors `multica/server/pkg/agent/args.go` (specifically the `filterCustomArgs`
//! logic from `claude.go`).

use std::collections::HashMap;
use tracing::warn;

/// Specifies whether a blocked arg takes a value or is standalone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockedArgMode {
    /// Flag takes a value (next arg or `=value`).
    WithValue,
    /// Flag is boolean, no value.
    Standalone,
    /// Flag may take the next non-flag arg or `=value`.
    OptionalValue,
}

/// Removes protocol-critical flags from user-configured custom args to prevent
/// breaking daemon↔agent communication. Each backend defines its own blocked set
/// (the flags it hardcodes).
pub fn filter_custom_args(args: &[String], blocked: &HashMap<&str, BlockedArgMode>) -> Vec<String> {
    if args.is_empty() {
        return vec![];
    }

    let mut filtered = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        let raw = &args[i];
        let arg = unshell_quote_arg(raw);
        let mut flag = arg.as_str();
        let mut has_inline_value = false;

        if let Some(idx) = arg.find('=') {
            flag = &arg[..idx];
            has_inline_value = true;
        }

        if let Some(mode) = blocked.get(flag) {
            warn!(
                "custom_args: blocked protocol-critical flag, skipping flag={}",
                flag
            );
            if *mode == BlockedArgMode::WithValue && !has_inline_value {
                // The next arg is the value for this flag — skip it too.
                i += 1;
            } else if *mode == BlockedArgMode::OptionalValue
                && !has_inline_value
                && i + 1 < args.len()
                && !unshell_quote_arg(&args[i + 1]).starts_with('-')
            {
                // Optional values are consumed only when the next token is not
                // another flag, so a boolean form cannot swallow an unrelated option.
                i += 1;
            }
            i += 1;
            continue;
        }

        filtered.push(arg);
        i += 1;
    }
    filtered
}

/// Strips a single layer of shell-style single or double quotes from an argument.
/// It handles two forms:
///
///   - `--flag='value'` or `--flag="value"` → `--flag=value`
///   - `'standalone'` or `"standalone"`     → `standalone`
///
/// Only flag-style args (`-x=…`, `--flag=…`) get inline value unquoting. Plain
/// assignment syntax like `model="o3"` is left alone.
pub fn unshell_quote_arg(arg: &str) -> String {
    if arg.starts_with('-') {
        if let Some(idx) = arg.find('=') {
            let value = &arg[idx + 1..];
            if let Some(unquoted) = strip_surrounding_quotes(value) {
                return format!("{}={}", &arg[..idx], unquoted);
            }
            return arg.to_string();
        }
    }
    if let Some(unquoted) = strip_surrounding_quotes(arg) {
        return unquoted.to_string();
    }
    arg.to_string()
}

fn strip_surrounding_quotes(s: &str) -> Option<&str> {
    if s.len() >= 2 {
        let first = s.chars().next().unwrap();
        let last = s.chars().last().unwrap();
        if (first == '\'' && last == '\'') || (first == '"' && last == '"') {
            return Some(&s[1..s.len() - 1]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unshell_quote_arg() {
        assert_eq!(unshell_quote_arg("--foo='bar'"), "--foo=bar");
        assert_eq!(unshell_quote_arg("--foo=\"bar\""), "--foo=bar");
        assert_eq!(unshell_quote_arg("'standalone'"), "standalone");
        assert_eq!(unshell_quote_arg("\"standalone\""), "standalone");
        assert_eq!(unshell_quote_arg("model=\"o3\""), "model=\"o3\""); // Plain assignments left alone
    }

    #[test]
    fn test_filter_custom_args() {
        let mut blocked = HashMap::new();
        blocked.insert("-p", BlockedArgMode::WithValue);
        blocked.insert("--profile", BlockedArgMode::WithValue);
        blocked.insert("--safe-mode", BlockedArgMode::Standalone);
        blocked.insert("-c", BlockedArgMode::OptionalValue);

        let args = vec![
            "-p".to_string(),
            "foo".to_string(),
            "--profile=bar".to_string(),
            "--safe-mode".to_string(),
            "-c".to_string(),
            "val".to_string(),
            "-c".to_string(),
            "-x".to_string(), // next is flag, shouldn't consume
            "--custom".to_string(),
            "value".to_string(),
        ];

        let filtered = filter_custom_args(&args, &blocked);
        assert_eq!(filtered, vec!["-x", "--custom", "value"]);
    }
}
