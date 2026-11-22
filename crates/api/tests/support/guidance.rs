use super::*;

pub(super) fn seed_guidance(server: &Server) -> Result<String> {
    let cwd = server.root.path().join("project 한글");
    let skill = cwd.join(".agents/skills/fixture-verify");
    fs::create_dir_all(skill.join("references"))?;
    fs::write(cwd.join("AGENTS.md"), "Use public fixture checks.")?;
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: fixture-verify\ndescription: Verify the synthetic fixture workflow.\n---\nRun the fixture checks.",
    )?;
    fs::write(skill.join("references/checks.md"), "Fixture reference material.")?;
    Ok("repo:.agents/skills/fixture-verify/SKILL.md".into())
}

pub(super) async fn shared_guidance_jobs() -> Result<()> {
    let server = Server::open_scenario(Provider::Codex, "workflow").await?;
    let id = seed_guidance(&server)?;
    for provider in PROVIDERS {
        server.login(provider, "default").await?;
        let name = provider_name(provider);
        let route = format!("/v1/providers/{name}/jobs");
        let request = json!({"text":"Verify the fixture","skills":[{"id":id}]});
        let enqueue = || {
            server
                .request(Method::POST, &route, "default")
                .header("Idempotency-Key", format!("guidance-{name}"))
                .json(&request)
                .send()
        };
        let first: Job = enqueue().await?.error_for_status()?.json().await?;
        let retry: Job = enqueue().await?.error_for_status()?.json().await?;
        ensure!(first.id == retry.id);
        server.wait(first.id, JobState::Waiting).await?;
        let (approval, _) = server.pending(first.id, 0).await?;
        server.respond(first.id, approval).await?;
        let completed = server.wait(first.id, JobState::Completed).await?;
        ensure!(completed.request.skills[0].id == id);
        let saved: SavedConversation = server
            .request(
                Method::GET,
                &format!("/v1/providers/{name}/conversations/{}", completed.job.conversation_id.unwrap()),
                "default",
            )
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(saved.messages[0].skills[0].id == id);
        ensure!(saved.messages[0].guidance.as_ref().unwrap().sources == ["repo:AGENTS.md"]);
    }
    let blocking = server.enqueue(Provider::Codex, "default", "Hold this profile", None).await?;
    let (approval, _) = server.pending(blocking.id, 0).await?;
    let queued: Job = server
        .request(Method::POST, "/v1/providers/codex/jobs", "default")
        .json(&json!({"text":"Use selected skill","skills":[{"id":id}]}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(queued.state == JobState::Queued);
    fs::remove_file(server.root.path().join("project 한글/.agents/skills/fixture-verify/SKILL.md"))?;
    server.respond(blocking.id, approval).await?;
    server.wait(blocking.id, JobState::Completed).await?;
    let failed = server.wait(queued.id, JobState::Failed).await?;
    ensure!(failed.job.error.as_deref().is_some_and(|error| error.contains("Selected skill is unavailable")));
    server.stop().await?;
    println!("ok: shared skills survive durable requests, idempotent retries and saved history on all providers; deleted queued skills fail closed");
    Ok(())
}
