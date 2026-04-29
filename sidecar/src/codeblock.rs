/// Streaming markdown codeblock detector.
///
/// Watches for ```sh, ```bash, ```shell, ```zsh, ```$ fences in a stream
/// of text deltas. When a complete fenced block is detected, emits the
/// contained command text.
pub struct CodeblockDetector {
    buffer: String,
    in_block: bool,
    fence_lang: String,
}

const SHELL_LANGS: &[&str] = &["sh", "bash", "shell", "zsh", "$", "console"];

impl CodeblockDetector {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            in_block: false,
            fence_lang: String::new(),
        }
    }

    /// Feed a text delta. Returns `Some(command)` if a complete shell
    /// codeblock was just closed.
    pub fn feed(&mut self, text: &str) -> Option<String> {
        self.buffer.push_str(text);

        loop {
            if !self.in_block {
                // Look for opening fence
                if let Some(fence_pos) = self.buffer.find("```") {
                    let after_fence = &self.buffer[fence_pos + 3..];
                    if let Some(newline_pos) = after_fence.find('\n') {
                        let lang = after_fence[..newline_pos].trim().to_lowercase();
                        if SHELL_LANGS.iter().any(|&s| lang == s || lang.starts_with(s)) {
                            self.in_block = true;
                            self.fence_lang = lang;
                            self.buffer = after_fence[newline_pos + 1..].to_string();
                            continue;
                        } else {
                            // Not a shell block, skip past this fence
                            self.buffer = after_fence[newline_pos + 1..].to_string();
                            continue;
                        }
                    }
                    // No newline yet — incomplete fence, wait for more data
                    break;
                }
                break;
            } else {
                // Look for closing fence
                if let Some(close_pos) = self.buffer.find("```") {
                    let command = self.buffer[..close_pos].trim().to_string();
                    self.buffer = self.buffer[close_pos + 3..].to_string();
                    self.in_block = false;
                    self.fence_lang.clear();

                    if !command.is_empty() {
                        // Strip leading $ or # prompts
                        let command = command
                            .lines()
                            .map(|l| {
                                let trimmed = l.trim_start();
                                if trimmed.starts_with("$ ") {
                                    &trimmed[2..]
                                } else if trimmed.starts_with("# ") {
                                    &trimmed[2..]
                                } else {
                                    l
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n");

                        return Some(command);
                    }
                    continue;
                }
                // No closing fence yet
                break;
            }
        }

        None
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
        self.in_block = false;
        self.fence_lang.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_sh_block() {
        let mut d = CodeblockDetector::new();
        assert_eq!(d.feed("Here's a command:\n```sh\n"), None);
        assert_eq!(d.feed("ls -la\n"), None);
        assert_eq!(d.feed("```\n"), Some("ls -la".to_string()));
    }

    #[test]
    fn strips_dollar_prompt() {
        let mut d = CodeblockDetector::new();
        d.feed("```bash\n$ git status\n```\n");
        // feed should return on the closing fence
        let mut result = None;
        for chunk in ["```bash\n", "$ git status\n", "```\n"] {
            if let Some(cmd) = d.feed(chunk) {
                result = Some(cmd);
            }
        }
        // Reset and test in one shot
        let mut d2 = CodeblockDetector::new();
        let r = d2.feed("```bash\n$ git status\n```\n");
        assert_eq!(r, Some("git status".to_string()));
    }

    #[test]
    fn ignores_non_shell_blocks() {
        let mut d = CodeblockDetector::new();
        assert_eq!(d.feed("```python\nprint('hi')\n```\n"), None);
    }
}
