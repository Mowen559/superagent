use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdrMetadata {
    pub id: String,
    pub number: usize,
    pub title: String,
    pub status: String,
    pub date: String,
    pub file_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdrDetail {
    pub metadata: AdrMetadata,
    pub context: String,
    pub decision: String,
    pub consequences: String,
    pub raw_content: String,
}

pub struct AdrService;

impl AdrService {
    fn get_adr_dir(project_root: &Path) -> PathBuf {
        let docs_adr = project_root.join("docs").join("adr");
        if docs_adr.exists() {
            return docs_adr;
        }
        project_root.join(".deepcloud").join("adr")
    }

    pub fn list_adrs(project_root: &Path) -> Result<Vec<AdrMetadata>, String> {
        let dir = Self::get_adr_dir(project_root);
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut adrs = Vec::new();
        let entries = fs::read_dir(&dir).map_err(|e| format!("Failed to read ADR dir: {}", e))?;

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("md") {
                let file_name = path.file_name().unwrap().to_string_lossy().to_string();
                if let Ok(content) = fs::read_to_string(&path) {
                    let meta = Self::parse_adr_metadata(&file_name, &content);
                    adrs.push(meta);
                }
            }
        }

        adrs.sort_by_key(|a| a.number);
        Ok(adrs)
    }

    pub fn get_adr(project_root: &Path, id_or_number: &str) -> Result<AdrDetail, String> {
        let dir = Self::get_adr_dir(project_root);
        if !dir.exists() {
            return Err("ADR directory does not exist".to_string());
        }

        let entries = fs::read_dir(&dir).map_err(|e| format!("Failed to read ADR dir: {}", e))?;
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let file_name = path.file_name().unwrap().to_string_lossy().to_string();
            if file_name.starts_with(id_or_number) || file_name.contains(id_or_number) {
                let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
                let metadata = Self::parse_adr_metadata(&file_name, &content);
                let (context, decision, consequences) = Self::extract_sections(&content);
                return Ok(AdrDetail {
                    metadata,
                    context,
                    decision,
                    consequences,
                    raw_content: content,
                });
            }
        }

        Err(format!("ADR '{}' not found", id_or_number))
    }

    pub fn create_adr(
        project_root: &Path,
        title: &str,
        status: &str,
        context: &str,
        decision: &str,
        consequences: &str,
    ) -> Result<AdrMetadata, String> {
        let dir = Self::get_adr_dir(project_root);
        fs::create_dir_all(&dir).map_err(|e| format!("Failed to create ADR dir: {}", e))?;

        let existing = Self::list_adrs(project_root).unwrap_or_default();
        let next_number = existing.iter().map(|a| a.number).max().unwrap_or(0) + 1;

        let slug: String = title
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        let clean_slug = slug.trim_matches('-');
        let file_name = format!("{:04}-{}.md", next_number, clean_slug);
        let target_path = dir.join(&file_name);

        let date_str = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let status_clean = if status.trim().is_empty() { "Accepted" } else { status.trim() };

        let markdown = format!(
            "# [ADR-{:04}] {}\n\n- **Status**: {}\n- **Date**: {}\n\n## Context\n{}\n\n## Decision\n{}\n\n## Consequences\n{}\n",
            next_number, title, status_clean, date_str, context, decision, consequences
        );

        fs::write(&target_path, &markdown).map_err(|e| format!("Failed to write ADR: {}", e))?;

        Ok(AdrMetadata {
            id: format!("{:04}", next_number),
            number: next_number,
            title: title.to_string(),
            status: status_clean.to_string(),
            date: date_str,
            file_name,
        })
    }

    fn parse_adr_metadata(file_name: &str, content: &str) -> AdrMetadata {
        let num_str: String = file_name.chars().take_while(|c| c.is_ascii_digit()).collect();
        let number = num_str.parse::<usize>().unwrap_or(0);
        let id = if number > 0 { format!("{:04}", number) } else { file_name.to_string() };

        let mut title = file_name.replace(".md", "");
        let mut status = "Proposed".to_string();
        let mut date = "Unknown".to_string();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("# ") {
                title = trimmed.trim_start_matches("# ").to_string();
            } else if trimmed.starts_with("- **Status**:") {
                status = trimmed.trim_start_matches("- **Status**:").trim().to_string();
            } else if trimmed.starts_with("- **Date**:") {
                date = trimmed.trim_start_matches("- **Date**:").trim().to_string();
            }
        }

        AdrMetadata {
            id,
            number,
            title,
            status,
            date,
            file_name: file_name.to_string(),
        }
    }

    fn extract_sections(content: &str) -> (String, String, String) {
        let mut context = String::new();
        let mut decision = String::new();
        let mut consequences = String::new();

        let mut current_sec = "";
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("## Context") {
                current_sec = "context";
                continue;
            } else if trimmed.starts_with("## Decision") {
                current_sec = "decision";
                continue;
            } else if trimmed.starts_with("## Consequences") {
                current_sec = "consequences";
                continue;
            }

            match current_sec {
                "context" => {
                    context.push_str(line);
                    context.push('\n');
                }
                "decision" => {
                    decision.push_str(line);
                    decision.push('\n');
                }
                "consequences" => {
                    consequences.push_str(line);
                    consequences.push('\n');
                }
                _ => {}
            }
        }

        (context.trim().to_string(), decision.trim().to_string(), consequences.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adr_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let meta = AdrService::create_adr(
            dir.path(),
            "Adopt LiteFlow for Engine",
            "Accepted",
            "Complex data pipeline need orchestration",
            "We decide to adopt LiteFlow 2.12",
            "Rules hot reloadable",
        )
        .unwrap();

        assert_eq!(meta.number, 1);
        assert!(meta.file_name.contains("adopt-liteflow"));

        let list = AdrService::list_adrs(dir.path()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "0001");

        let detail = AdrService::get_adr(dir.path(), "0001").unwrap();
        assert_eq!(detail.metadata.status, "Accepted");
        assert!(detail.decision.contains("LiteFlow"));
    }
}

