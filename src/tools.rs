use serde_json::json;
pub const RUN_COMMAND_TOOL: &str = "run_command";
pub fn definitions() -> serde_json::Value {
    json!([{"type":"function","function":{"name":RUN_COMMAND_TOOL,"description":"Run a terminal command after user approval. Prefer a simple executable with arguments. Set shell=true only when shell syntax such as pipes or redirects is required.","parameters":{"type":"object","properties":{"command":{"type":"string"},"reason":{"type":"string"},"shell":{"type":"boolean","description":"Use the user's shell for pipes, redirects, or shell operators."}},"required":["command","reason"]}}}])
}
