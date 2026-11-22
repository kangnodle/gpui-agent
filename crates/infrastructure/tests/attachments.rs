use anyhow::Result;
use gpui_agent_application::ports::JobRepository;
use gpui_agent_contracts::{Attachment,
                           ChatRequest,
                           JobScope,
                           Provider};
use gpui_agent_domain::attachment::{MAX_IMAGE_BYTES,
                                    MAX_SCOPE_UPLOADS,
                                    MAX_UPLOADS};
use gpui_agent_infrastructure::{JobStore,
                                Settings};
use serde_json::json;

fn text(name: &str, value: &str) -> Attachment {
    Attachment::Text {
        name: name.into(),
        text: value.into(),
        source: None,
    }
}

#[test]
fn uploaded_snapshots_survive_restart_and_scope_limits_do_not_block_identical_retries() -> Result<()> {
    let root = tempfile::tempdir()?;
    let settings = Settings::discover(root.path().into(), Some(root.path().join("home")))?;
    let workspaces = settings.workspaces(settings.application()?)?;
    let path = root.path().canonicalize()?.display().to_string();
    let scope = JobScope {
        provider: Provider::Codex,
        account_id: "default".into(),
        workspace_id: Some(workspaces.default_workspace().unwrap()),
        workspace_path: path,
    };
    let store = JobStore::open(settings.clone(), workspaces.clone())?;
    let first = store.upload_attachment(&scope, &text("first.txt", "immutable"))?;
    for index in 1 .. MAX_SCOPE_UPLOADS {
        store.upload_attachment(&scope, &text(&format!("{index}.txt"), "value"))?;
    }
    assert!(store.upload_attachment(&scope, &text("overflow.txt", "value")).is_err());
    assert_eq!(store.upload_attachment(&scope, &text("first.txt", "immutable"))?.id, first.id);
    let scopes = MAX_UPLOADS / MAX_SCOPE_UPLOADS;
    for index in 1 .. scopes {
        let account = store.create_account(Provider::Codex, &format!("account {index}"))?;
        let other = JobScope {
            account_id: account.id,
            ..scope.clone()
        };
        for item in 0 .. MAX_SCOPE_UPLOADS {
            store.upload_attachment(&other, &text(&format!("{item}.txt"), "value"))?;
        }
        assert!(store.delete_attachment(&other, first.id).is_err());
        let request: ChatRequest = serde_json::from_value(json!({"text":""}))?;
        assert!(store.enqueue_uploaded_once(&other, &request, &[first.id], "foreign-upload").is_err());
    }
    let account = store.create_account(Provider::Codex, "last account")?;
    assert!(
        store
            .upload_attachment(
                &JobScope {
                    account_id: account.id,
                    ..scope.clone()
                },
                &text("global.txt", "overflow")
            )
            .is_err()
    );
    drop(store);
    // Given a reopened host, the stable upload ID still admits the original
    // immutable snapshot.
    let store = JobStore::open(settings, workspaces)?;
    assert_eq!(store.uploaded_attachments(&scope)?.len(), MAX_SCOPE_UPLOADS);
    let request: ChatRequest = serde_json::from_value(json!({"text":""}))?;
    let job = store.enqueue_uploaded_once(&scope, &request, &[first.id], "restart-upload")?;
    assert_eq!(store.get(job.id)?.request.attachments, vec![text("first.txt", "immutable")]);
    store.delete_attachment(&scope, first.id)?;
    assert_eq!(store.enqueue_uploaded_once(&scope, &request, &[first.id], "restart-upload")?.id, job.id);
    assert!(store.enqueue_uploaded_once(&scope, &request, &[first.id], "new-upload-message").is_err());
    Ok(())
}

#[test]
fn invalid_or_oversized_delivery_cannot_create_a_job_or_reserve_its_command_id() -> Result<()> {
    let root = tempfile::tempdir()?;
    let settings = Settings::discover(root.path().into(), Some(root.path().join("home")))?;
    let workspaces = settings.workspaces(settings.application()?)?;
    let scope = JobScope {
        provider: Provider::Codex,
        account_id: "default".into(),
        workspace_id: Some(workspaces.default_workspace().unwrap()),
        workspace_path: root.path().canonicalize()?.display().to_string(),
    };
    let store = JobStore::open(settings, workspaces)?;
    let mut bytes = vec![0; MAX_IMAGE_BYTES];
    bytes[.. 8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    let image = Attachment::from_bytes("large.png".into(), &bytes).map_err(anyhow::Error::msg)?;
    let upload = store.upload_attachment(&scope, &image)?;
    let mut request: ChatRequest = serde_json::from_value(json!({"text":"","attachments":[{"type":"text","name":"a.txt","text":"x"}]}))?;
    assert!(store.enqueue_uploaded_once(&scope, &request, &[upload.id, upload.id], "aggregate").is_err());
    assert!(store.list()?.is_empty());
    request.attachments.clear();
    for ids in [vec![0], vec![-1], vec![upload.id; 9]] {
        assert!(store.enqueue_uploaded_once(&scope, &request, &ids, "invalid-ids").is_err());
    }
    let job = store.enqueue_uploaded_once(&scope, &request, &[upload.id], "aggregate")?;
    assert_eq!(store.get(job.id)?.request.attachments, vec![image]);
    for invalid in [
        text("../escape", "data"),
        text("binary.txt", "a\0b"),
        Attachment::Image {
            name: "bad.png".into(),
            mime_type: "image/png".into(),
            data: "invalid!".into(),
        },
    ] {
        assert!(store.upload_attachment(&scope, &invalid).is_err());
    }
    assert_eq!(store.uploaded_attachments(&scope)?.len(), 1);
    Ok(())
}
