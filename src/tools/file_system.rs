use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::fs;

use crate::executor::Risk;

fn path_of(args: &Value) -> String {
    args.get("path")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string()
}

fn flag(args: &Value, key: &str) -> bool {
    args.get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// `read_file`: print a file's contents for the model.
pub struct ReadFile;

#[async_trait]
impl super::Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Read a file's contents. For large files pass offset/max_bytes to read in chunks instead of the whole file."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file to read." },
                "offset": { "type": "integer", "description": "Byte offset to start reading from (default 0)." },
                "max_bytes": { "type": "integer", "description": "Maximum bytes to read (default 20000)." },
            },
            "required": ["path"],
        })
    }

    fn risk(&self, _args: &Value) -> Risk {
        Risk::Safe
    }

    fn preview(&self, args: &Value) -> String {
        format!("Read file: {}", path_of(args))
    }

    async fn execute(&self, args: &Value) -> Result<String> {
        let path = path_of(args);
        let bytes = fs::read(&path).with_context(|| format!("could not read `{path}`"))?;
        let offset = args
            .get("offset")
            .and_then(|value| value.as_i64())
            .unwrap_or(0)
            .max(0) as usize;
        let max_bytes = args
            .get("max_bytes")
            .and_then(|value| value.as_i64())
            .unwrap_or(20_000)
            .max(1) as usize;
        let truncated = offset + max_bytes < bytes.len();
        let end = (offset + max_bytes).min(bytes.len());
        let slice = bytes.get(offset..end).unwrap_or(&[]);
        let mut contents = crate::security::redact(&String::from_utf8_lossy(slice));
        if truncated {
            contents.push_str("\n[output truncated; read on from a later offset]");
        }
        Ok(contents)
    }
}

/// `write_file`: create, overwrite or append to a file.
pub struct WriteFile;

#[async_trait]
impl super::Tool for WriteFile {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
        "Create a file with the given contents, overwriting it unless append=true. Creates parent directories as needed."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path of the file to write." },
                "content": { "type": "string", "description": "Full contents to write." },
                "append": { "type": "boolean", "description": "Append to the file instead of overwriting." },
            },
            "required": ["path", "content"],
        })
    }

    fn risk(&self, _args: &Value) -> Risk {
        Risk::Caution
    }

    fn preview(&self, args: &Value) -> String {
        let action = if flag(args, "append") {
            "Append"
        } else {
            "Write"
        };
        let bytes = args
            .get("content")
            .and_then(|value| value.as_str())
            .map(|content| content.len())
            .unwrap_or(0);
        format!("{action} {bytes} bytes to: {}", path_of(args))
    }

    async fn execute(&self, args: &Value) -> Result<String> {
        let path = path_of(args);
        let content = args
            .get("content")
            .and_then(|value| value.as_str())
            .context("write_file requires a string `content`")?;
        if let Some(parent) = std::path::Path::new(&path).parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("could not create parent directory for `{path}`"))?;
        }
        if flag(args, "append") {
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .with_context(|| format!("could not open `{path}` for appending"))?;
            file.write_all(content.as_bytes())
                .with_context(|| format!("could not append to `{path}`"))?;
            Ok(format!("Appended {} bytes to {path}.", content.len()))
        } else {
            fs::write(&path, content).with_context(|| format!("could not write `{path}`"))?;
            Ok(format!("Wrote {} bytes to {path}.", content.len()))
        }
    }
}

/// `list_dir`: list the entries of a directory.
pub struct ListDir;

#[async_trait]
impl super::Tool for ListDir {
    fn name(&self) -> &'static str {
        "list_dir"
    }

    fn description(&self) -> &'static str {
        "List the files and subdirectories inside a directory, one entry per line with type and size."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path of the directory to list." },
                "include_hidden": { "type": "boolean", "description": "Also list dotfiles (default false)." },
            },
            "required": ["path"],
        })
    }

    fn risk(&self, _args: &Value) -> Risk {
        Risk::Safe
    }

    fn preview(&self, args: &Value) -> String {
        format!("List directory: {}", path_of(args))
    }

    async fn execute(&self, args: &Value) -> Result<String> {
        let path = path_of(args);
        let include_hidden = flag(args, "include_hidden");
        let entries = fs::read_dir(&path)
            .with_context(|| format!("could not read directory `{path}`"))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut lines: Vec<String> = Vec::new();
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            if !include_hidden && name.starts_with('.') {
                continue;
            }
            let metadata = entry.metadata().ok();
            let kind = match metadata.as_ref().map(|metadata| metadata.is_dir()) {
                Some(true) => format!("{name}/ (dir)"),
                _ => {
                    let size = metadata.map(|metadata| metadata.len()).unwrap_or(0);
                    format!("{name} ({size} bytes)")
                }
            };
            lines.push(kind);
        }
        lines.sort();
        if lines.is_empty() {
            return Ok("Directory is empty.".into());
        }
        lines.truncate(1_000);
        Ok(lines.join("\n"))
    }
}

/// `delete`: remove a file or directory.
pub struct DeletePath;

#[async_trait]
impl super::Tool for DeletePath {
    fn name(&self) -> &'static str {
        "delete"
    }

    fn description(&self) -> &'static str {
        "Delete a file, or a directory (set recursive=true to remove a non-empty directory)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path of the file or directory to delete." },
                "recursive": { "type": "boolean", "description": "Remove a non-empty directory tree." },
            },
            "required": ["path"],
        })
    }

    fn risk(&self, _args: &Value) -> Risk {
        Risk::Dangerous
    }

    fn preview(&self, args: &Value) -> String {
        let suffix = if flag(args, "recursive") {
            " (recursive)"
        } else {
            ""
        };
        format!("Delete: {}{suffix}", path_of(args))
    }

    async fn execute(&self, args: &Value) -> Result<String> {
        let path = path_of(args);
        let metadata =
            fs::symlink_metadata(&path).with_context(|| format!("could not stat `{path}`"))?;
        let recursive = flag(args, "recursive");
        if metadata.is_dir() {
            if recursive {
                fs::remove_dir_all(&path).with_context(|| format!("could not delete `{path}`"))?;
            } else {
                fs::remove_dir(&path).with_context(|| format!("could not delete `{path}`"))?;
            }
        } else {
            fs::remove_file(&path).with_context(|| format!("could not delete `{path}`"))?;
        }
        Ok(format!("Deleted {path}."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;
    use serde_json::json;

    fn args(path: &str) -> Value {
        json!({ "path": path })
    }

    #[tokio::test]
    async fn write_read_roundtrip_with_append() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("note.txt");
        let file = file.to_string_lossy().to_string();

        let write = WriteFile;
        write
            .execute(&json!({ "path": file, "content": "first" }))
            .await
            .unwrap();
        write
            .execute(&json!({ "path": file, "content": "second", "append": true }))
            .await
            .unwrap();

        let read = ReadFile;
        let contents = read.execute(&args(&file)).await.unwrap();
        assert_eq!(contents, "firstsecond");

        let list = ListDir;
        let listing = list
            .execute(&args(&dir.path().to_string_lossy()))
            .await
            .unwrap();
        assert!(listing.contains("note.txt"));
    }

    #[tokio::test]
    async fn delete_removes_file_and_non_empty_dir_recursively() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tmp.txt");
        fs::write(&file, "data").unwrap();
        let delete = DeletePath;
        delete
            .execute(&args(&file.to_string_lossy()))
            .await
            .unwrap();
        assert!(!file.exists());

        let sub = dir.path().join("tree");
        fs::create_dir_all(sub.join("nested")).unwrap();
        fs::write(sub.join("nested").join("x"), "y").unwrap();
        delete
            .execute(&json!({ "path": sub.to_string_lossy(), "recursive": true }))
            .await
            .unwrap();
        assert!(!sub.exists());
    }

    #[test]
    fn file_tools_are_risky() {
        assert_eq!(WriteFile.risk(&Value::Null), Risk::Caution);
        assert_eq!(DeletePath.risk(&Value::Null), Risk::Dangerous);
        assert_eq!(ReadFile.risk(&Value::Null), Risk::Safe);
        assert_eq!(ListDir.risk(&Value::Null), Risk::Safe);
    }
}
