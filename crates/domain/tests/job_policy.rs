use gpui_agent_domain::{Attachment,
                        ChatRequest,
                        QueueError,
                        validate_queue_capacity};

fn request() -> ChatRequest {
    ChatRequest {
        extensions: None,
        skills: vec![],
        computer_use: false,
        text: "original question".into(),
        model: Some("saved-model".into()),
        session_id: None,
        conversation_id: Some(7),
        attachments: vec![],
        claude_permission_mode: None,
        codex_execution_policy: None,
    }
}

#[test]
fn given_queue_counts_when_admitting_work_then_host_and_account_limits_are_independent() {
    assert!(validate_queue_capacity(255, 31).is_ok());
    assert_eq!(validate_queue_capacity(256, 31), Err(QueueError::Full));
    assert_eq!(validate_queue_capacity(255, 32), Err(QueueError::AccountFull));
    assert_eq!(validate_queue_capacity(256, 32), Err(QueueError::Full));
}

#[test]
fn given_an_attachment_only_request_when_projected_then_preview_uses_its_name() {
    let mut request = request();
    assert_eq!(request.preview(), "original question");
    request.text = "  \n".into();
    request.attachments.push(Attachment::Text {
        name: "notes.txt".into(),
        text: "contents".into(),
        source: None,
    });
    assert_eq!(request.preview(), "notes.txt");
    request.attachments.clear();
    assert_eq!(request.preview(), "Attachment");
}
