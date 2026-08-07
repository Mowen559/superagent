import re

with open('src/services/agent/skills_sync.rs', 'r', encoding='utf-8') as f:
    content = f.read()

# Replace normalize_skill_dir_name
old_normalize = """fn normalize_skill_dir_name(value: &str) -> String {
    value
        .trim()
        .trim_matches('/')
        .trim_matches('\\\\')
        .replace(['/', '\\\\', ':'], "-")
}"""

new_normalize = """fn normalize_skill_dir_name(value: &str) -> String {
    let normalized = value
        .trim()
        .trim_matches('/')
        .trim_matches('\\\\')
        .replace(['/', '\\\\', ':'], "-");
    
    // Ensure we don't return path traversal sequences like "." or ".."
    if normalized == "." || normalized == ".." {
        return "invalid-skill-name".to_string();
    }
    normalized
}"""

# Check if it has \\ or \\\
# Let's just use Python script to replace it without dealing with backslashes
