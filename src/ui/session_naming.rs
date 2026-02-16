use chrono::{FixedOffset, Utc};
use uuid::Uuid;

/// Asia/Seoul (KST, UTC+9) 타임존 기준 오늘 날짜를 `YYYYMMDD` 형식으로 반환한다.
pub fn today_date_string() -> String {
    let kst = FixedOffset::east_opt(9 * 3600).expect("valid KST offset");
    Utc::now().with_timezone(&kst).format("%Y%m%d").to_string()
}

/// UUID v4 문자열을 생성하여 세션 식별자로 반환한다.
pub fn generate_session_id() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_date_string_has_valid_format() {
        let date = today_date_string();
        assert_eq!(date.len(), 8);
        assert!(date.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn generate_session_id_is_valid_uuid_v4() {
        let id = generate_session_id();
        let parsed = Uuid::parse_str(&id).expect("valid UUID");
        assert_eq!(parsed.get_version(), Some(uuid::Version::Random));
    }

    #[test]
    fn generate_session_id_is_unique() {
        let id1 = generate_session_id();
        let id2 = generate_session_id();
        assert_ne!(id1, id2);
    }
}
