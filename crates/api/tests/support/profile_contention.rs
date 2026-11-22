use super::*;

// Given an authenticated status read holding the native profile, when a job
// starts, then it waits observably and reaches approval after the read
// finishes.
pub(super) async fn run() -> Result<()> {
    scenario(false).await?;
    scenario(true).await?;
    for provider in PROVIDERS {
        status_polling(provider).await?;
    }
    println!("ok: concurrent native status reads preserve job approval, completion and cancellation");
    Ok(())
}

async fn status_polling(provider: Provider) -> Result<()> {
    let server = Server::open(provider).await?;
    server.login(provider, "default").await?;
    let work = async {
        for _ in 0 .. 8 {
            let job = server.enqueue(provider, "default", "Hold this profile", None).await?;
            let (approval, _) = server.pending(job.id, 0).await?;
            server.respond(job.id, approval).await?;
            server.wait(job.id, JobState::Completed).await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    let poll = async {
        loop {
            if let Err(error) = server
                .request(Method::GET, &format!("/v1/providers/{}/status", provider_name(provider)), "default")
                .send()
                .await
            {
                break Err::<(), _>(error);
            }
            tokio::task::yield_now().await;
        }
    };
    let result = tokio::select! { result = work => result, result = poll => result.map_err(anyhow::Error::from) };
    server.stop().await?;
    result
}

async fn scenario(cancel: bool) -> Result<()> {
    let server = Server::open(Provider::Codex).await?;
    server.login(Provider::Codex, "default").await?;
    let home = server.root.path().join("home/codex");
    fs::write(home.join("hold-status"), "")?;
    let status = tokio::spawn(server.request(Method::GET, "/v1/providers/codex/status", "default").send());
    let result = async {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !home.join("status-ready").exists() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await?;
        let job = server.enqueue(Provider::Codex, "default", "Hold this profile", None).await?;
        let mut response = server
            .request(Method::GET, &format!("/v1/jobs/{}/events", job.id), "default")
            .send()
            .await?
            .error_for_status()?;
        let mut buffer = Vec::new();
        let mut waiting = false;
        while !waiting {
            while let Some(end) = buffer.iter().position(|b| *b == b'\n') {
                let event: JobEvent = serde_json::from_slice(&buffer.drain(..= end).collect::<Vec<_>>())?;
                if matches!(event.event, StreamEvent::Agent(Event::RuntimeStatus { code: Some(ref code), .. }) if code == "profile_busy") {
                    waiting = true;
                    break;
                }
            }
            if !waiting {
                let Some(chunk) = response.chunk().await? else {
                    let failed = server.job(job.id).await?.job;
                    let busy = failed
                        .error
                        .as_deref()
                        .is_some_and(|message| message.contains("another operation is using this provider profile"));
                    anyhow::bail!("Missing interaction: job={} state={:?} profile_busy={busy}", job.id, failed.state);
                };
                buffer.extend(chunk);
            }
        }
        if cancel {
            server
                .request(Method::DELETE, &format!("/v1/jobs/{}", job.id), "default")
                .send()
                .await?
                .error_for_status()?;
            server.wait(job.id, JobState::Cancelled).await?;
            let events = server
                .request(Method::GET, &format!("/v1/jobs/{}/events", job.id), "default")
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            for line in events.lines() {
                let event: JobEvent = serde_json::from_str(line)?;
                ensure!(
                    !matches!(event.event, StreamEvent::Interaction { .. }),
                    "A cancelled profile wait must not request approval"
                );
            }
        } else {
            fs::write(home.join("release-status"), "")?;
            let (approval, _) = server.pending(job.id, 0).await?;
            server.respond(job.id, approval).await?;
            server.wait(job.id, JobState::Completed).await?;
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    fs::write(home.join("release-status"), "")?;
    status.await??.error_for_status()?;
    server.stop().await?;
    result?;
    Ok(())
}
