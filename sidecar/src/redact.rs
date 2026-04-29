/// Redacts sensitive patterns from text before sending to an LLM provider.
///
/// Default patterns: password, token, api_key, secret, credential
/// Users can add custom patterns via config.

const DEFAULT_PATTERNS: &[&str] = &[
    "password",
    "passwd",
    "token",
    "api_key",
    "api-key",
    "apikey",
    "secret",
    "credential",
    "private_key",
    "private-key",
];

pub fn redact(text: &str, extra_patterns: &[String]) -> String {
    let mut result = text.to_string();

    let all_patterns: Vec<&str> = DEFAULT_PATTERNS
        .iter()
        .copied()
        .chain(extra_patterns.iter().map(|s| s.as_str()))
        .collect();

    for pattern in &all_patterns {
        // Case-insensitive search for lines containing the pattern
        // followed by `=` or `:` (assignment patterns)
        let lower = result.to_lowercase();
        let pat_lower = pattern.to_lowercase();

        let mut redacted = String::new();
        for line in result.lines() {
            let line_lower = line.to_lowercase();
            if line_lower.contains(&pat_lower) {
                // Check if it looks like an assignment (key=value or key: value)
                if line.contains('=') || line.contains(':') {
                    // Redact the value portion
                    if let Some(eq_pos) = line.find('=') {
                        redacted.push_str(&line[..eq_pos + 1]);
                        redacted.push_str("[REDACTED]");
                    } else if let Some(col_pos) = line.find(':') {
                        redacted.push_str(&line[..col_pos + 1]);
                        redacted.push_str(" [REDACTED]");
                    } else {
                        redacted.push_str(line);
                    }
                } else {
                    redacted.push_str(line);
                }
            } else {
                redacted.push_str(line);
            }
            redacted.push('\n');
        }

        // Remove trailing newline if original didn't have one
        if !text.ends_with('\n') && redacted.ends_with('\n') {
            redacted.pop();
        }

        result = redacted;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_password_assignment() {
        let input = "export DB_PASSWORD=mysecret123\nother_line=fine";
        let output = redact(input, &[]);
        assert!(output.contains("DB_PASSWORD=[REDACTED]"));
        assert!(output.contains("other_line=fine"));
    }

    #[test]
    fn redacts_api_key() {
        let input = "ANTHROPIC_API_KEY=sk-ant-12345";
        let output = redact(input, &[]);
        assert!(output.contains("API_KEY=[REDACTED]"));
        assert!(!output.contains("sk-ant-12345"));
    }

    #[test]
    fn preserves_non_sensitive_lines() {
        let input = "ls -la\ntotal 42\ndrwxr-xr-x 5 user staff 160 Apr 28";
        let output = redact(input, &[]);
        assert_eq!(output, input);
    }

    #[test]
    fn custom_patterns() {
        let input = "MY_CUSTOM_KEY=should_hide";
        let output = redact(input, &["MY_CUSTOM".to_string()]);
        assert!(output.contains("[REDACTED]"));
    }
}
