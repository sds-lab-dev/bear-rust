use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileKind {
    Spec,
    Plan,
}

#[derive(Debug, Deserialize)]
pub struct FileValidationResponse {
    pub valid: bool,
    pub reason: String,
}

pub fn validation_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "valid": {
                "type": "boolean",
                "description": "Whether the file is a valid document of the expected kind"
            },
            "reason": {
                "type": "string",
                "description": "Explanation of why the file is valid or invalid"
            }
        },
        "required": ["valid", "reason"],
        "additionalProperties": false
    })
}

pub fn system_prompt() -> &'static str {
    r#"You are a file validation assistant. Your task is to read the given file and determine whether it is a valid document of the specified kind. You MUST read the file using the Read tool before making a judgment. Respond with a JSON object indicating validity and reasoning."#
}

const SPEC_VALIDATION_PROMPT_TEMPLATE: &str = r#"Read the following file and determine whether it is a valid software specification document.

A valid specification document should contain most of the following elements:
- Overview or summary of the system being specified
- Requirements (functional and/or non-functional)
- Goals, scope, or acceptance criteria
- Structured sections describing what the system must do

The file does NOT need to follow an exact template. If the document is clearly describing software requirements or a system specification in a reasonable format, consider it valid.

File path:
{{FILE_PATH}}

Output MUST be valid JSON conforming to the provided JSON Schema.
Output MUST contain ONLY the JSON object, with no extra text."#;

const PLAN_VALIDATION_PROMPT_TEMPLATE: &str = r#"Read the following file and determine whether it is a valid software development plan document.

A valid development plan document should contain most of the following elements:
- A list of implementation tasks or work items
- Description of each task's scope or purpose
- Dependency information or execution order between tasks
- Structured sections describing how the implementation will proceed

The file does NOT need to follow an exact template. If the document is clearly describing a software implementation plan with identifiable tasks, consider it valid.

File path:
{{FILE_PATH}}

Output MUST be valid JSON conforming to the provided JSON Schema.
Output MUST contain ONLY the JSON object, with no extra text."#;

pub fn build_validation_prompt(file_path: &Path, kind: FileKind) -> String {
    let template = match kind {
        FileKind::Spec => SPEC_VALIDATION_PROMPT_TEMPLATE,
        FileKind::Plan => PLAN_VALIDATION_PROMPT_TEMPLATE,
    };
    template.replace("{{FILE_PATH}}", &file_path.display().to_string())
}

/// 파일 경로를 로컬에서 검증한다. 상대 경로는 `base_dir` 기준으로 해석한다.
/// 성공 시 절대 경로를 반환하고, 실패 시 한국어 에러 메시지를 반환한다.
pub fn validate_file_locally(raw_path: &str, base_dir: &Path) -> Result<PathBuf, String> {
    let path = PathBuf::from(raw_path);
    let absolute_path = if path.is_absolute() {
        path
    } else {
        let joined = base_dir.join(&path);
        fs::canonicalize(&joined).map_err(|_| {
            format!(
                "파일이 존재하지 않습니다: {}",
                joined.display()
            )
        })?
    };

    if !absolute_path.exists() {
        return Err(format!(
            "파일이 존재하지 않습니다: {}",
            absolute_path.display()
        ));
    }

    if !absolute_path.is_file() {
        return Err(format!(
            "일반 파일이 아닙니다: {}",
            absolute_path.display()
        ));
    }

    let metadata = fs::metadata(&absolute_path).map_err(|err| {
        format!(
            "파일 정보를 읽을 수 없습니다: {} ({})",
            absolute_path.display(),
            err
        )
    })?;

    if metadata.len() == 0 {
        return Err(format!(
            "파일이 비어 있습니다: {}",
            absolute_path.display()
        ));
    }

    Ok(absolute_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn validation_schema_is_valid_json() {
        let schema = validation_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["valid"].is_object());
        assert!(schema["properties"]["reason"].is_object());
    }

    #[test]
    fn deserialize_valid_response() {
        let json = serde_json::json!({
            "valid": true,
            "reason": "The file contains a valid specification."
        });
        let response: FileValidationResponse = serde_json::from_value(json).unwrap();
        assert!(response.valid);
    }

    #[test]
    fn deserialize_invalid_response() {
        let json = serde_json::json!({
            "valid": false,
            "reason": "The file does not contain any requirements."
        });
        let response: FileValidationResponse = serde_json::from_value(json).unwrap();
        assert!(!response.valid);
    }

    #[test]
    fn build_spec_validation_prompt_contains_path() {
        let path = Path::new("/workspace/spec.md");
        let prompt = build_validation_prompt(path, FileKind::Spec);
        assert!(prompt.contains("/workspace/spec.md"));
        assert!(prompt.contains("specification"));
    }

    #[test]
    fn build_plan_validation_prompt_contains_path() {
        let path = Path::new("/workspace/plan.md");
        let prompt = build_validation_prompt(path, FileKind::Plan);
        assert!(prompt.contains("/workspace/plan.md"));
        assert!(prompt.contains("development plan"));
    }

    #[test]
    fn validate_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        let result = validate_file_locally("/nonexistent/file.md", tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("존재하지 않습니다"));
    }

    #[test]
    fn validate_directory_not_file() {
        let tmp = TempDir::new().unwrap();
        let dir_path = tmp.path().join("subdir");
        fs::create_dir(&dir_path).unwrap();
        let result = validate_file_locally(
            &dir_path.display().to_string(),
            tmp.path(),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("일반 파일이 아닙니다"));
    }

    #[test]
    fn validate_empty_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("empty.md");
        fs::write(&file_path, "").unwrap();
        let result = validate_file_locally(
            &file_path.display().to_string(),
            tmp.path(),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("비어 있습니다"));
    }

    #[test]
    fn validate_valid_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("spec.md");
        fs::write(&file_path, "# Specification\nSome content").unwrap();
        let result = validate_file_locally(
            &file_path.display().to_string(),
            tmp.path(),
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), file_path);
    }

    #[test]
    fn validate_relative_path() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("docs").join("spec.md");
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(&file_path, "# Spec content").unwrap();

        let result = validate_file_locally("docs/spec.md", tmp.path());
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.is_absolute());
        assert!(resolved.ends_with("docs/spec.md"));
    }

    #[test]
    fn validate_relative_path_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let result = validate_file_locally("nonexistent/file.md", tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("존재하지 않습니다"));
    }

    #[test]
    fn system_prompt_is_nonempty() {
        assert!(!system_prompt().is_empty());
    }
}
