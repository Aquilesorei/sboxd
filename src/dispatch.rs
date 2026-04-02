pub fn command_string(argv: &[String]) -> String {
    argv.join(" ")
}

pub fn matches(pattern: &str, command: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == command;
    }

    let starts_with_wildcard = pattern.starts_with('*');
    let ends_with_wildcard = pattern.ends_with('*');
    let parts: Vec<&str> = pattern.split('*').filter(|part| !part.is_empty()).collect();

    if parts.is_empty() {
        return true;
    }

    let mut cursor = 0usize;

    for (index, part) in parts.iter().enumerate() {
        if index == 0 && !starts_with_wildcard {
            if !command[cursor..].starts_with(part) {
                return false;
            }
            cursor += part.len();
            continue;
        }

        match command[cursor..].find(part) {
            Some(found) => cursor += found + part.len(),
            None => return false,
        }
    }

    if !ends_with_wildcard {
        let last = parts.last().expect("parts is not empty");
        return command.ends_with(last);
    }

    true
}

#[cfg(test)]
mod tests {
    use super::{command_string, matches};

    #[test]
    fn joins_command_tokens_with_single_spaces() {
        let command = vec![
            "cargo".to_string(),
            "build".to_string(),
            "--release".to_string(),
        ];
        assert_eq!(command_string(&command), "cargo build --release");
    }

    #[test]
    fn exact_match_requires_full_string_equality() {
        assert!(matches("npm install", "npm install"));
        assert!(!matches("npm install", "npm install lodash"));
    }

    #[test]
    fn wildcard_match_handles_prefix_suffix_and_infix() {
        assert!(matches("cargo build*", "cargo build --release"));
        assert!(matches("*pytest", "python -m pytest"));
        assert!(matches("python * pytest", "python -m pytest"));
        assert!(!matches("cargo build*", "cargo test"));
    }
}
