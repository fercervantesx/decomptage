use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DangerLevel {
    #[serde(rename = "safe")]
    Safe,
    #[serde(rename = "caution")]
    Caution,
    #[serde(rename = "destructive")]
    Destructive,
}

#[derive(Debug, Clone)]
pub enum ToolCall {
    SuggestCommand {
        id: String,
        command: String,
        explanation: String,
        danger_level: DangerLevel,
    },
    RunCommand {
        id: String,
        command: String,
        explanation: String,
        wait_for_output: bool,
    },
    ReadTerminal {
        id: String,
        lines: Option<u32>,
    },
    ReadFile {
        id: String,
        path: String,
        lines: Option<u32>,
    },
}

impl ToolCall {
    pub fn id(&self) -> &str {
        match self {
            Self::SuggestCommand { id, .. } => id,
            Self::RunCommand { id, .. } => id,
            Self::ReadTerminal { id, .. } => id,
            Self::ReadFile { id, .. } => id,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::SuggestCommand { .. } => "suggest_command",
            Self::RunCommand { .. } => "run_command",
            Self::ReadTerminal { .. } => "read_terminal",
            Self::ReadFile { .. } => "read_file",
        }
    }

    pub fn danger_level(&self) -> DangerLevel {
        match self {
            Self::SuggestCommand { danger_level, .. } => danger_level.clone(),
            Self::RunCommand { command, .. } => classify_command_danger(command),
            Self::ReadTerminal { .. } => DangerLevel::Safe,
            Self::ReadFile { .. } => DangerLevel::Safe,
        }
    }

    pub fn needs_approval(&self) -> bool {
        matches!(self, Self::RunCommand { .. })
    }

    /// Parse a tool call from provider-agnostic representation
    pub fn from_raw(id: &str, name: &str, args: &Value) -> Option<Self> {
        match name {
            "suggest_command" => Some(Self::SuggestCommand {
                id: id.to_string(),
                command: args.get("command")?.as_str()?.to_string(),
                explanation: args
                    .get("explanation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                danger_level: args
                    .get("danger_level")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or(DangerLevel::Safe),
            }),
            "run_command" => Some(Self::RunCommand {
                id: id.to_string(),
                command: args.get("command")?.as_str()?.to_string(),
                explanation: args
                    .get("explanation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                wait_for_output: args
                    .get("wait_for_output")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
            }),
            "read_terminal" => Some(Self::ReadTerminal {
                id: id.to_string(),
                lines: args.get("lines").and_then(|v| v.as_u64()).map(|v| v as u32),
            }),
            "read_file" => Some(Self::ReadFile {
                id: id.to_string(),
                path: args.get("path")?.as_str()?.to_string(),
                lines: args.get("lines").and_then(|v| v.as_u64()).map(|v| v as u32),
            }),
            _ => None,
        }
    }
}

/// Returns the tool definitions in Anthropic format (also works for OpenAI with minor transforms)
pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "suggest_command",
            "description": "Suggest a shell command for the user to run. The command will appear as an actionable card in the UI. The user decides whether to insert it into their terminal.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to suggest"
                    },
                    "explanation": {
                        "type": "string",
                        "description": "Brief explanation of what this command does"
                    },
                    "danger_level": {
                        "type": "string",
                        "enum": ["safe", "caution", "destructive"],
                        "description": "How dangerous this command is. 'destructive' for rm, git reset --hard, etc."
                    }
                },
                "required": ["command", "explanation"]
            }
        }),
        json!({
            "name": "run_command",
            "description": "Execute a shell command in the user's terminal. Requires user approval. The output will be returned to you. Use this when you need to see command output to proceed (e.g., checking file contents, running diagnostics).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command to execute"
                    },
                    "explanation": {
                        "type": "string",
                        "description": "Why you need to run this command"
                    },
                    "wait_for_output": {
                        "type": "boolean",
                        "description": "Whether to wait for the command to complete and return output. Default true."
                    }
                },
                "required": ["command", "explanation"]
            }
        }),
        json!({
            "name": "read_terminal",
            "description": "Read the current visible content of the user's terminal. Use this to check command output or see what's on screen.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "lines": {
                        "type": "integer",
                        "description": "Number of lines to read. If omitted, reads the full visible screen."
                    }
                }
            }
        }),
        json!({
            "name": "read_file",
            "description": "Read the contents of a file from the filesystem. Only files within the current working directory are accessible without approval.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file (relative to cwd or absolute)"
                    },
                    "lines": {
                        "type": "integer",
                        "description": "Maximum number of lines to read. If omitted, reads the full file (capped at 500 lines)."
                    }
                },
                "required": ["path"]
            }
        }),
    ]
}

/// Convert tool definitions to OpenAI function-calling format
pub fn tool_definitions_openai() -> Vec<Value> {
    tool_definitions()
        .into_iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool["name"],
                    "description": tool["description"],
                    "parameters": tool["input_schema"]
                }
            })
        })
        .collect()
}

/// Heuristic to classify command danger level
fn classify_command_danger(command: &str) -> DangerLevel {
    let destructive_patterns = [
        "rm -rf", "rm -r", "rmdir", "git reset --hard", "git clean -fd",
        "git push --force", "git push -f", "dd if=", "mkfs", "format",
        "> /dev/", "chmod -R 777", "sudo rm",
    ];
    let caution_patterns = [
        "sudo", "kill", "pkill", "docker rm", "docker rmi",
        "npm uninstall", "pip uninstall", "brew uninstall",
    ];

    let lower = command.to_lowercase();
    for pat in &destructive_patterns {
        if lower.contains(pat) {
            return DangerLevel::Destructive;
        }
    }
    for pat in &caution_patterns {
        if lower.contains(pat) {
            return DangerLevel::Caution;
        }
    }
    DangerLevel::Safe
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_rm_rf_as_destructive() {
        assert!(matches!(
            classify_command_danger("rm -rf /tmp/test"),
            DangerLevel::Destructive
        ));
    }

    #[test]
    fn classifies_sudo_as_caution() {
        assert!(matches!(
            classify_command_danger("sudo apt install vim"),
            DangerLevel::Caution
        ));
    }

    #[test]
    fn classifies_ls_as_safe() {
        assert!(matches!(
            classify_command_danger("ls -la"),
            DangerLevel::Safe
        ));
    }

    #[test]
    fn parses_suggest_command() {
        let args = json!({"command": "ls", "explanation": "list files"});
        let tc = ToolCall::from_raw("123", "suggest_command", &args).unwrap();
        assert_eq!(tc.name(), "suggest_command");
        assert!(!tc.needs_approval());
    }

    #[test]
    fn parses_run_command() {
        let args = json!({"command": "npm install", "explanation": "install deps"});
        let tc = ToolCall::from_raw("456", "run_command", &args).unwrap();
        assert_eq!(tc.name(), "run_command");
        assert!(tc.needs_approval());
    }
}
