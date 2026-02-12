use std::path::Path;

use chrono::{FixedOffset, Utc};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SessionNameResponse {
    pub session_name: String,
}

pub fn session_name_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "session_name": {
                "type": "string",
                "description": "A short session name consisting of 1-5 English words joined by hyphens. Only lowercase letters (a-z), digits (0-9), hyphens (-), and underscores (_) are allowed."
            }
        },
        "required": ["session_name"],
        "additionalProperties": false
    })
}

const SESSION_NAME_PROMPT: &str = r#"Generate a short session name for a development task based on the user's requirement below.

Rules:
- The name MUST consist of 1 to 5 English words joined by hyphens (-)
- Only use lowercase letters (a-z), digits (0-9), hyphens (-), and underscores (_)
- Do NOT use spaces, uppercase letters, or any other characters
- The name should briefly describe the main topic of the requirement
- Keep it concise and descriptive

Example names: "user-auth-system", "cli-tool-refactor", "api-rate-limiting", "file-upload"

Output MUST be valid JSON conforming to the provided JSON Schema.
Output MUST contain ONLY the JSON object, with no extra text.

User requirement:
<<<
{{REQUIREMENTS}}
>>>"#;

pub fn build_session_name_prompt(requirements: &str) -> String {
    SESSION_NAME_PROMPT.replace("{{REQUIREMENTS}}", requirements)
}

/// 모델이 생성한 세션 이름을 디렉토리 이름으로 사용할 수 있도록 정제한다.
pub fn sanitize_session_name(raw: &str) -> String {
    let lowered: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();

    let mut result = String::new();
    let mut prev_was_hyphen = false;
    for c in lowered.chars() {
        if c == '-' {
            if !prev_was_hyphen {
                result.push(c);
            }
            prev_was_hyphen = true;
        } else {
            result.push(c);
            prev_was_hyphen = false;
        }
    }

    let result = result.trim_matches('-').to_string();

    if result.is_empty() {
        "unnamed-session".to_string()
    } else {
        result
    }
}

/// Asia/Seoul (KST, UTC+9) 타임존 기준 오늘 날짜를 `YYYYMMDD` 형식으로 반환한다.
pub fn today_date_string() -> String {
    let kst = FixedOffset::east_opt(9 * 3600).expect("valid KST offset");
    Utc::now().with_timezone(&kst).format("%Y%m%d").to_string()
}

/// 워크스페이스의 `.bear/<date_dir>/` 디렉토리 안에 동일한 이름이 이미 존재하면
/// `-2`, `-3`, ... 접미사를 붙여 고유한 이름을 반환한다.
pub fn ensure_unique_name(workspace: &Path, date_dir: &str, base_name: &str) -> String {
    let bear_date_dir = workspace.join(".bear").join(date_dir);

    if !bear_date_dir.join(base_name).exists() {
        return base_name.to_string();
    }

    for suffix in 2.. {
        let candidate = format!("{}-{}", base_name, suffix);
        if !bear_date_dir.join(&candidate).exists() {
            return candidate;
        }
    }

    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_valid_json() {
        let schema = session_name_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["session_name"].is_object());
    }

    #[test]
    fn prompt_contains_requirements() {
        let prompt = build_session_name_prompt("Build a REST API");
        assert!(prompt.contains("Build a REST API"));
    }

    #[test]
    fn deserialize_session_name_response() {
        let json = serde_json::json!({ "session_name": "rest-api-build" });
        let response: SessionNameResponse = serde_json::from_value(json).unwrap();
        assert_eq!(response.session_name, "rest-api-build");
    }

    #[test]
    fn sanitize_valid_name() {
        assert_eq!(sanitize_session_name("user-auth-system"), "user-auth-system");
    }

    #[test]
    fn sanitize_uppercase_to_lowercase() {
        assert_eq!(sanitize_session_name("User-Auth-System"), "user-auth-system");
    }

    #[test]
    fn sanitize_spaces_to_hyphens() {
        assert_eq!(sanitize_session_name("user auth system"), "user-auth-system");
    }

    #[test]
    fn sanitize_collapses_consecutive_hyphens() {
        assert_eq!(sanitize_session_name("user--auth---system"), "user-auth-system");
    }

    #[test]
    fn sanitize_strips_leading_trailing_hyphens() {
        assert_eq!(sanitize_session_name("-user-auth-"), "user-auth");
    }

    #[test]
    fn sanitize_special_characters() {
        assert_eq!(sanitize_session_name("user@auth!system"), "user-auth-system");
    }

    #[test]
    fn sanitize_empty_returns_fallback() {
        assert_eq!(sanitize_session_name(""), "unnamed-session");
    }

    #[test]
    fn sanitize_only_special_chars_returns_fallback() {
        assert_eq!(sanitize_session_name("@!#$"), "unnamed-session");
    }

    #[test]
    fn sanitize_preserves_underscores() {
        assert_eq!(sanitize_session_name("my_session_name"), "my_session_name");
    }

    #[test]
    fn sanitize_preserves_digits() {
        assert_eq!(sanitize_session_name("v2-api-update"), "v2-api-update");
    }

    #[test]
    fn today_date_string_has_valid_format() {
        let date = today_date_string();
        assert_eq!(date.len(), 8);
        assert!(date.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn unique_name_when_no_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            ensure_unique_name(tmp.path(), "20250101", "my-session"),
            "my-session"
        );
    }

    #[test]
    fn unique_name_appends_suffix_on_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".bear/20250101/my-session")).unwrap();
        assert_eq!(
            ensure_unique_name(tmp.path(), "20250101", "my-session"),
            "my-session-2"
        );
    }

    #[test]
    fn unique_name_skips_existing_suffixes() {
        let tmp = tempfile::tempdir().unwrap();
        let date_dir = tmp.path().join(".bear/20250101");
        std::fs::create_dir_all(date_dir.join("my-session")).unwrap();
        std::fs::create_dir_all(date_dir.join("my-session-2")).unwrap();
        std::fs::create_dir_all(date_dir.join("my-session-3")).unwrap();
        assert_eq!(
            ensure_unique_name(tmp.path(), "20250101", "my-session"),
            "my-session-4"
        );
    }
}
