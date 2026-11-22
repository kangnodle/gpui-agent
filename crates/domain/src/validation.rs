use crate::Provider;

pub fn validate_login(provider: Provider, device_code: bool) -> Result<(), &'static str> {
    if device_code && provider != Provider::Codex {
        return Err("Device-code login is only supported by Codex");
    }
    Ok(())
}

pub fn validate_chat(text: &str, session_id: Option<&str>) -> Result<(), &'static str> {
    if text.trim().is_empty() || text.len() > 256 * 1024 {
        return Err("Message must contain 1–262144 bytes of nonempty text");
    }
    if session_id.is_some_and(|id| id.is_empty() || id.len() > 512) {
        return Err("Invalid session ID");
    }
    Ok(())
}

pub fn validate_message(text: &str, session_id: Option<&str>, attachments: &[crate::Attachment]) -> Result<(), &'static str> {
    crate::validate_attachments(attachments)?;
    validate_chat(
        if text.trim().is_empty() && !attachments.is_empty() {
            "attachment"
        } else {
            text
        },
        session_id,
    )?;
    if text.len() > 256 * 1024 {
        return Err("Message exceeds 256 KiB");
    }
    Ok(())
}

pub fn validate_search(query: &str) -> Result<(), &'static str> {
    if query.chars().count() > 256 {
        return Err("Search must contain at most 256 characters");
    }
    Ok(())
}

pub fn normalize_title(title: &str) -> Result<&str, &'static str> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 100 || title.chars().any(char::is_control) {
        return Err("Title must contain 1–100 characters without control characters");
    }
    Ok(title)
}

pub fn conversation_title(text: &str) -> String { text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(60).collect() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_preserves_byte_and_character_limits() {
        assert!(validate_chat(" \n", None).is_err());
        assert!(validate_chat(&"x".repeat(256 * 1024), None).is_ok());
        assert!(validate_chat(&"x".repeat(256 * 1024 + 1), None).is_err());
        assert!(validate_chat("hello", Some("")).is_err());
        assert!(validate_chat("hello", Some(&"x".repeat(512))).is_ok());
        assert!(validate_chat("hello", Some(&"x".repeat(513))).is_err());
        assert_eq!(normalize_title("  안녕 🦀  "), Ok("안녕 🦀"));
        assert!(normalize_title(&"한".repeat(100)).is_ok());
        assert!(normalize_title(&"한".repeat(101)).is_err());
        assert!(normalize_title("a\nb").is_err());
        assert!(normalize_title("  ").is_err());
        assert!(validate_search(&"🦀".repeat(256)).is_ok());
        assert!(validate_search(&"🦀".repeat(257)).is_err());
        for provider in crate::PROVIDERS {
            assert!(validate_login(provider, false).is_ok());
            assert_eq!(validate_login(provider, true).is_ok(), provider == Provider::Codex);
        }
        assert_eq!(conversation_title("  hello \n 세계  "), "hello 세계");
        assert_eq!(conversation_title(&"🦀".repeat(70)), "🦀".repeat(60));
    }

    #[test]
    fn attachments_allow_empty_text_without_bypassing_message_and_session_limits() {
        let attachments = [crate::Attachment::Text {
            name: "context.txt".into(),
            text: "context".into(),
            source: None,
        }];
        assert!(validate_message("", None, &[]).is_err());
        assert!(validate_message(" \n", None, &attachments).is_ok());
        assert!(validate_message("", Some(""), &attachments).is_err());
        assert!(validate_message(&" ".repeat(256 * 1024), None, &attachments).is_ok());
        assert_eq!(
            validate_message(&" ".repeat(256 * 1024 + 1), None, &attachments),
            Err("Message exceeds 256 KiB")
        );
        assert!(
            validate_message(
                "text",
                None,
                &[crate::Attachment::Text {
                    name: "../bad".into(),
                    text: "context".into(),
                    source: None
                }]
            )
            .is_err()
        );
    }
}
