use anyhow::Result;
use async_trait::async_trait;
use fancy_regex::Regex;
use serde_json::{json, Value};
use std::path::Path;

use crate::executor::Risk;

/// Search raw lines of text in a directory tree with a regular expression,
/// a safe scanning alternative to piping `grep` through the shell.
pub struct Search;

const DEFAULT_MAX_MATCHES: usize = 200;
const MAX_SKIPPED_BYTES: u64 = 2 * 1024 * 1024;

fn path_of(args: &Value) -> String {
    args.get("path")
        .and_then(|value| value.as_str())
        .unwrap_or(".")
        .to_string()
}

fn pattern_of(args: &Value) -> String {
    args.get("pattern")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string()
}

fn int_arg(args: &Value, key: &str) -> Option<usize> {
    args.get(key)
        .and_then(|value| value.as_i64())
        .map(|value| value as usize)
}

fn flag(args: &Value, key: &str) -> bool {
    args.get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

#[async_trait]
impl super::Tool for Search {
    fn name(&self) -> &'static str {
        "search"
    }

    fn description(&self) -> &'static str {
        "Search file contents for a regex pattern (e.g. \"fn \\w+\"). Returns matching lines as path:line: text, capped at 200 matches. Provide a directory for a recursive search of text files, or a single file path. For plain text lookups prefer this over piping grep through the shell."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File or directory to search. Defaults to the current directory.",
                },
                "pattern": { "type": "string", "description": "Regex to match against each line." },
                "case_insensitive": { "type": "boolean", "description": "Ignore case for ASCII characters." },
                "max_matches": { "type": "integer", "description": "Stop after this many matches (default 200)." },
            },
            "required": ["pattern"],
        })
    }

    fn risk(&self, _args: &Value) -> Risk {
        Risk::Safe
    }

    fn preview(&self, args: &Value) -> String {
        format!("Search {:?} for /{}/", path_of(args), pattern_of(args))
    }

    async fn execute(&self, args: &Value) -> Result<String> {
        let root = path_of(args);
        let pattern = pattern_of(args);
        let case_insensitive = flag(args, "case_insensitive");
        let max_matches = int_arg(args, "max_matches").unwrap_or(DEFAULT_MAX_MATCHES);
        let pattern = if case_insensitive {
            format!("(?i:{pattern})")
        } else {
            pattern
        };
        let regex =
            Regex::new(&pattern).map_err(|error| anyhow::anyhow!("invalid pattern: {error}"))?;

        let mut paths: Vec<String> = Vec::new();
        let path = Path::new(&root);
        if path.is_dir() {
            collect_files(path, &mut paths, 0, 16)?;
        } else {
            paths.push(root.clone());
        }
        paths.sort();

        let mut matches: Vec<String> = Vec::new();
        for file in &paths {
            if matches.len() >= max_matches {
                break;
            }
            let bytes = match std::fs::read(file) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            if bytes.len() as u64 > MAX_SKIPPED_BYTES {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            for (line_number, line) in text.lines().enumerate() {
                if regex.is_match(line).unwrap_or(false) {
                    matches.push(format!("{file}:{}: {}", line_number + 1, line.trim()));
                    if matches.len() >= max_matches {
                        break;
                    }
                }
            }
        }

        if matches.is_empty() {
            return Ok(format!(
                "No matches for `/{}` in {}.",
                pattern_of(args),
                root
            ));
        }
        let mut out = matches.join("\n");
        if out.len() > 20_000 {
            out.truncate(20_000);
            out.push_str("\n[output truncated]");
        }
        Ok(out)
    }
}

fn collect_files(dir: &Path, out: &mut Vec<String>, depth: usize, max_depth: usize) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            if depth < max_depth {
                collect_files(&path, out, depth + 1, max_depth)?;
            }
        } else if !is_binary(&path) {
            out.push(path.to_string_lossy().to_string());
        }
    }
    Ok(())
}

fn is_binary(path: &Path) -> bool {
    std::fs::read(path)
        .ok()
        .map(|bytes| bytes.contains(&0))
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;
    use serde_json::json;
    use std::fs;

    #[tokio::test]
    async fn finds_regex_matches_recursively() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello world\nfn main() {}\n").unwrap();
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("b.txt"), "fn helper()\n").unwrap();
        fs::write(dir.path().join("sub").join("ignored.bin"), "fn \x00x\n").unwrap();

        let out = Search
            .execute(&json!({
                "path": dir.path().to_string_lossy(),
                "pattern": r"fn \w+",
            }))
            .await
            .unwrap();
        assert!(out.contains("a.txt"), "a.txt missing from {out}");
        assert!(out.contains("b.txt"), "b.txt missing from {out}");
        assert!(!out.contains("fn \u{0}x"), "binary file should be skipped");
    }

    #[tokio::test]
    async fn respects_max_matches_and_case_insensitivity() {
        let dir = tempfile::tempdir().unwrap();
        let mut contents = String::new();
        for i in 0..50 {
            contents.push_str(&format!("line {i} WITH token\n"));
        }
        fs::write(dir.path().join("log.txt"), contents).unwrap();

        let out = Search
            .execute(&json!({
                "path": dir.path().to_string_lossy(),
                "pattern": "with token",
                "case_insensitive": true,
                "max_matches": 10,
            }))
            .await
            .unwrap();
        assert_eq!(out.lines().count(), 10);
    }

    #[test]
    fn search_is_safe() {
        assert_eq!(Search.risk(&Value::Null), Risk::Safe);
    }
}
