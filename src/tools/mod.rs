pub mod file_system;
pub mod search;
pub mod shell;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

use crate::executor::Risk;

pub const RUN_COMMAND_TOOL: &str = "run_command";

/// Every tool exposes the same contract to the agent. The agent knows nothing
/// about individual tools; it asks the registry which tool to run, checks its
/// risk for approval, and reports the result. Adding a tool means adding one
/// more `impl Tool` (usually in its own module) plus one registry entry.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique name used in tool calls (e.g. `run_command`).
    fn name(&self) -> &'static str;
    /// Human-readable description shown to the model.
    fn description(&self) -> &'static str;
    /// JSON schema for the tool arguments.
    fn parameters(&self) -> serde_json::Value;
    /// How risky a specific invocation is, so the approval policy can apply.
    fn risk(&self, args: &serde_json::Value) -> Risk;
    /// One human-readable line describing what the call would do.
    fn preview(&self, args: &serde_json::Value) -> String;
    /// Perform the action and return its result text.
    async fn execute(&self, args: &serde_json::Value) -> Result<String>;
}

/// All registered tools. Extend the app by adding a tool here.
pub fn tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(shell::ShellTool),
        Box::new(file_system::ReadFile),
        Box::new(file_system::WriteFile),
        Box::new(file_system::ListDir),
        Box::new(file_system::DeletePath),
        Box::new(search::Search),
    ]
}

pub fn get(name: &str) -> Option<Box<dyn Tool>> {
    tools().into_iter().find(|tool| tool.name() == name)
}

/// OpenAI-style tool schema for every registered tool, as sent to providers.
pub fn definitions() -> serde_json::Value {
    json!(tools()
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name(),
                    "description": tool.description(),
                    "parameters": tool.parameters(),
                }
            })
        })
        .collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_include_every_registered_tool() {
        let names: Vec<&str> = tools().iter().map(|tool| tool.name()).collect();
        let schema = definitions();
        let listed: Vec<&str> = schema
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["function"]["name"].as_str())
            .collect();
        assert_eq!(listed.len(), names.len());
        for name in &names {
            assert!(listed.contains(name), "missing {name} in definitions");
        }
    }

    #[test]
    fn registry_looks_up_tools_by_name() {
        assert!(get(RUN_COMMAND_TOOL).is_some());
        assert!(get("read_file").is_some());
        assert!(get("write_file").is_some());
        assert!(get("list_dir").is_some());
        assert!(get("delete").is_some());
        assert!(get("search").is_some());
        assert!(get("does_not_exist").is_none());
    }
}
