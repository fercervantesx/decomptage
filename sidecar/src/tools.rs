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
        start_line: Option<u32>,
        end_line: Option<u32>,
    },
    WriteFile {
        id: String,
        path: String,
        content: String,
    },
    EditFile {
        id: String,
        path: String,
        old_string: String,
        new_string: String,
    },
    ListDirectory {
        id: String,
        path: Option<String>,
        recursive: bool,
    },
    SearchFiles {
        id: String,
        pattern: String,
        path: Option<String>,
        file_pattern: Option<String>,
    },
    WebFetch {
        id: String,
        url: String,
        max_length: Option<u32>,
    },
}

impl ToolCall {
    pub fn id(&self) -> &str {
        match self {
            Self::SuggestCommand { id, .. } => id,
            Self::RunCommand { id, .. } => id,
            Self::ReadTerminal { id, .. } => id,
            Self::ReadFile { id, .. } => id,
            Self::WriteFile { id, .. } => id,
            Self::EditFile { id, .. } => id,
            Self::ListDirectory { id, .. } => id,
            Self::SearchFiles { id, .. } => id,
            Self::WebFetch { id, .. } => id,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::SuggestCommand { .. } => "suggest_command",
            Self::RunCommand { .. } => "run_command",
            Self::ReadTerminal { .. } => "read_terminal",
            Self::ReadFile { .. } => "read_file",
            Self::WriteFile { .. } => "write_file",
            Self::EditFile { .. } => "edit_file",
            Self::ListDirectory { .. } => "list_directory",
            Self::SearchFiles { .. } => "search_files",
            Self::WebFetch { .. } => "web_fetch",
        }
    }

    pub fn danger_level(&self) -> DangerLevel {
        match self {
            Self::SuggestCommand { danger_level, .. } => danger_level.clone(),
            Self::RunCommand { command, .. } => classify_command_danger(command),
            Self::WriteFile { .. } => DangerLevel::Caution,
            Self::EditFile { .. } => DangerLevel::Caution,
            Self::ReadTerminal { .. } | Self::ReadFile { .. } |
            Self::ListDirectory { .. } | Self::SearchFiles { .. } |
            Self::WebFetch { .. } => DangerLevel::Safe,
        }
    }

    pub fn needs_approval(&self) -> bool {
        matches!(self, Self::RunCommand { .. } | Self::WriteFile { .. } | Self::EditFile { .. })
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
                start_line: args.get("start_line").and_then(|v| v.as_u64()).map(|v| v as u32),
                end_line: args.get("end_line").and_then(|v| v.as_u64()).map(|v| v as u32),
            }),
            "write_file" => Some(Self::WriteFile {
                id: id.to_string(),
                path: args.get("path")?.as_str()?.to_string(),
                content: args.get("content")?.as_str()?.to_string(),
            }),
            "edit_file" => Some(Self::EditFile {
                id: id.to_string(),
                path: args.get("path")?.as_str()?.to_string(),
                old_string: args.get("old_string")?.as_str()?.to_string(),
                new_string: args.get("new_string")?.as_str()?.to_string(),
            }),
            "list_directory" => Some(Self::ListDirectory {
                id: id.to_string(),
                path: args.get("path").and_then(|v| v.as_str()).map(|s| s.to_string()),
                recursive: args.get("recursive").and_then(|v| v.as_bool()).unwrap_or(false),
            }),
            "search_files" => Some(Self::SearchFiles {
                id: id.to_string(),
                pattern: args.get("pattern")?.as_str()?.to_string(),
                path: args.get("path").and_then(|v| v.as_str()).map(|s| s.to_string()),
                file_pattern: args.get("file_pattern").and_then(|v| v.as_str()).map(|s| s.to_string()),
            }),
            "web_fetch" => Some(Self::WebFetch {
                id: id.to_string(),
                url: args.get("url")?.as_str()?.to_string(),
                max_length: args.get("max_length").and_then(|v| v.as_u64()).map(|v| v as u32),
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
            "description": "Execute a shell command in the user's terminal. Requires user approval. The output will be returned to you. Use this when you need to see command output to proceed.",
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
            "description": "Read the current visible content of the user's terminal screen.",
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
            "description": "Read the contents of a file. Use for source code, configs, docs, READMEs, etc.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file (relative to cwd or absolute)"
                    },
                    "start_line": {
                        "type": "integer",
                        "description": "Line to start reading from (1-based). Omit to start from beginning."
                    },
                    "end_line": {
                        "type": "integer",
                        "description": "Line to stop reading at (inclusive). Omit to read to end (capped at 1000 lines)."
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "write_file",
            "description": "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Use for creating new files or completely rewriting existing ones.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to write to"
                    },
                    "content": {
                        "type": "string",
                        "description": "The full content to write"
                    }
                },
                "required": ["path", "content"]
            }
        }),
        json!({
            "name": "edit_file",
            "description": "Make a targeted edit to a file by replacing a specific string with new content. The old_string must match exactly (including whitespace).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to edit"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "The exact string to find and replace (must be unique in the file)"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The string to replace it with"
                    }
                },
                "required": ["path", "old_string", "new_string"]
            }
        }),
        json!({
            "name": "list_directory",
            "description": "List files and directories at a path. Returns names with type indicators (/ for dirs).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path to list. Defaults to current directory."
                    },
                    "recursive": {
                        "type": "boolean",
                        "description": "If true, list recursively (max 3 levels deep). Default false."
                    }
                }
            }
        }),
        json!({
            "name": "search_files",
            "description": "Search for a pattern in files. Like grep. Returns matching lines with file paths and line numbers.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The text or regex pattern to search for"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory or file to search in. Defaults to current directory."
                    },
                    "file_pattern": {
                        "type": "string",
                        "description": "Glob pattern for file names to include (e.g., '*.ts', '*.py')"
                    }
                },
                "required": ["pattern"]
            }
        }),
        json!({
            "name": "web_fetch",
            "description": "Fetch content from a URL. Useful for reading documentation, API references, package info, etc. Returns the text content (HTML tags stripped).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch"
                    },
                    "max_length": {
                        "type": "integer",
                        "description": "Maximum characters to return. Default 10000."
                    }
                },
                "required": ["url"]
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
