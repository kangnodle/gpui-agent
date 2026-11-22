use super::{conflict,
            failure,
            invalid};
use crate::git;
use gpui_agent_application::ApplicationResult;
use gpui_agent_domain::{GitCommand,
                        GitVersion,
                        PullRequest};
use std::{io::Write,
          path::Path,
          process::{Command,
                    Stdio}};
pub(super) fn repository(url: &str) -> ApplicationResult<String> {
    let path = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("git@github.com:"))
        .ok_or_else(|| invalid("PR creation supports github.com HTTPS or SSH remotes"))?;
    let path = path.trim_end_matches('/').strip_suffix(".git").unwrap_or(path.trim_end_matches('/'));
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() != 2
        || parts
            .iter()
            .any(|s| s.is_empty() || *s == "." || *s == ".." || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)))
    {
        return Err(invalid("Invalid GitHub repository remote"));
    }
    Ok(path.into())
}
fn output(out: git::Output) -> ApplicationResult<String> {
    if out.code != 0 {
        return Err(failure(format!("PR tool: {}", out.error.trim())));
    }
    if out.truncated {
        return Err(failure("PR tool output is too large"));
    }
    Ok(out.text().trim().to_owned())
}
fn url_valid(url: &str, repo: &str) -> bool {
    url.to_ascii_lowercase()
        .strip_prefix(&format!("https://github.com/{}/pull/", repo.to_ascii_lowercase()))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}
pub(super) fn run(cwd: &Path, hooks: &Path, command: &GitCommand, version: &GitVersion) -> ApplicationResult<PullRequest> {
    perform(
        command,
        version,
        |args| git::run(cwd, hooks, args),
        |args| {
            let exe = std::env::var_os("gpui_agent_GH_EXECUTABLE").unwrap_or_else(|| if cfg!(windows) { "gh.exe".into() } else { "gh".into() });
            let mut process = Command::new(exe);
            process
                .current_dir(cwd)
                .args(args)
                .env("GH_HOST", "github.com")
                .env("GH_PROMPT_DISABLED", "1")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env_remove("GH_REPO")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            git::execute(process)
        },
    )
}
fn perform(
    command: &GitCommand,
    version: &GitVersion,
    mut git: impl FnMut(&[&str]) -> ApplicationResult<git::Output>,
    mut gh: impl FnMut(&[&str]) -> ApplicationResult<git::Output>,
) -> ApplicationResult<PullRequest> {
    let (remote, base) = match command {
        | GitCommand::PreviewPr {
            remote,
            base,
        }
        | GitCommand::CreatePr {
            remote,
            base,
            ..
        } => (remote, base),
        | _ => unreachable!(),
    };
    for value in [remote, base] {
        if value.is_empty() || value.len() > 256 || value.starts_with('-') || value.contains('\0') {
            return Err(invalid("Invalid PR remote or base branch"));
        }
    }
    output(git(&["check-ref-format", "--branch", base])?)?;
    let remotes = output(git(&["remote"])?)?;
    if !remotes.lines().any(|name| name == remote) {
        return Err(invalid("Select an existing Git remote"));
    }
    let push_urls = output(git(&["remote", "get-url", "--push", "--all", remote])?)?;
    if push_urls.lines().count() != 1 {
        return Err(invalid("Select a remote with exactly one push URL"));
    }
    let repo = repository(&push_urls)?;
    if output(git(&["remote", "get-url", remote])?)? != push_urls {
        return Err(invalid("Fetch and push repository must match"));
    }
    let branch = output(git(&["symbolic-ref", "--short", "HEAD"])?)?;
    if branch == *base {
        return Err(invalid("PR head and base branches must differ"));
    }
    output(git(&["check-ref-format", "--branch", &branch])?)?;
    let head = version.head.as_deref().ok_or_else(|| invalid("Commit before creating a PR"))?;
    output(git(&[
        "fetch",
        "--no-tags",
        "--no-recurse-submodules",
        &push_urls,
        &format!("refs/heads/{base}"),
    ])?)?;
    let base_oid = output(git(&["rev-parse", "--verify", "FETCH_HEAD"])?)?;
    let range = format!("{base_oid}..{head}");
    let commits = output(git(&["log", "--format=%h %s", &range, "--"])?)?;
    if commits.is_empty() {
        return Err(invalid("There are no commits to propose"));
    }
    let diff = git(&["diff", "--no-ext-diff", "--no-textconv", "--no-color", &format!("{base_oid}...{head}"), "--"])?;
    if diff.code != 0 {
        return Err(failure(diff.error));
    }
    let published = output(git(&["ls-remote", "--heads", &push_urls, &format!("refs/heads/{branch}")])?)?;
    let needs_push = published.split_whitespace().next() != Some(head);
    let prs = output(gh(&[
        "pr", "list", "--repo", &repo, "--head", &branch, "--base", base, "--state", "open", "--json", "url",
    ])?)?;
    let prs: Vec<serde_json::Value> = serde_json::from_str(&prs).map_err(|e| failure(e.to_string()))?;
    let mut url = prs.first().and_then(|v| v["url"].as_str()).map(str::to_owned);
    if url.as_ref().is_some_and(|u| !url_valid(u, &repo)) {
        return Err(failure("GitHub CLI returned an invalid PR URL"));
    }
    if let GitCommand::CreatePr {
        title,
        body,
        draft,
        push,
        expected,
        expected_base,
        expected_branch,
        expected_repository,
        ..
    } = command
    {
        if expected != version || expected_base != &base_oid || expected_branch != &branch || expected_repository != &repo {
            return Err(conflict("HEAD or base changed; preview the PR again"));
        }
        if title.trim().is_empty() || title.len() > 256 || title.contains(['\r', '\n', '\0']) || body.len() > 65536 || body.contains('\0') {
            return Err(invalid("Invalid PR title or body"));
        }
        if needs_push && !push {
            return Err(conflict("Branch must be pushed; explicitly enable push and confirm"));
        }
        if needs_push {
            output(git(&[
                "push",
                "--porcelain",
                "--no-verify",
                "--recurse-submodules=no",
                "--",
                &push_urls,
                &format!("{head}:refs/heads/{branch}"),
            ])?)?;
        }
        if url.is_none() {
            let mut file = tempfile::NamedTempFile::new().map_err(|e| failure(e.to_string()))?;
            file.write_all(body.as_bytes()).map_err(|e| failure(e.to_string()))?;
            let path = file.path().to_str().ok_or_else(|| invalid("PR body path must be UTF-8"))?;
            let mut args = vec![
                "pr",
                "create",
                "--repo",
                &repo,
                "--base",
                base,
                "--head",
                &branch,
                "--title",
                title,
                "--body-file",
                path,
            ];
            if *draft {
                args.push("--draft");
            }
            let created = output(gh(&args)?)?;
            if !url_valid(&created, &repo) {
                return Err(failure("PR may have been created but no valid URL was returned; preview before retrying"));
            }
            url = Some(created);
        }
    }
    Ok(PullRequest {
        repository: repo,
        remote: remote.clone(),
        branch,
        base: base.clone(),
        base_oid,
        commits,
        diff: diff.text(),
        truncated: diff.truncated,
        needs_push: needs_push && matches!(command, GitCommand::PreviewPr { .. }),
        url,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn reply(text: &str) -> git::Output {
        git::Output {
            code: 0,
            bytes: text.as_bytes().to_vec(),
            error: String::new(),
            truncated: false,
        }
    }
    #[test]
    fn rejects_stale_reviews_and_unapproved_push_and_reuses_existing_pr() {
        let version = GitVersion {
            branch: None,
            head: Some("head".into()),
            index: "index".into(),
            worktree: "tree".into(),
        };
        for scenario in ["push_denied", "stale_base", "stale_branch", "stale_repo", "existing", "gh_failure"] {
            let command = GitCommand::CreatePr {
                remote: "origin".into(),
                base: "main".into(),
                title: "Fix".into(),
                body: "body".into(),
                draft: true,
                push: scenario != "push_denied",
                expected: version.clone(),
                expected_base: if scenario == "stale_base" { "old" } else { "base" }.into(),
                expected_branch: if scenario == "stale_branch" { "old" } else { "feature/test" }.into(),
                expected_repository: if scenario == "stale_repo" { "old/repo" } else { "owner/repo" }.into(),
            };
            let mut pushed = false;
            let mut created = false;
            let result = perform(
                &command,
                &version,
                |args| {
                    Ok(reply(match args[0] {
                        | "remote" if args.len() == 1 => "origin",
                        | "remote" => "https://github.com/owner/repo.git",
                        | "symbolic-ref" => "feature/test",
                        | "rev-parse" => "base",
                        | "log" => "head fix",
                        | "diff" => "+change",
                        | "push" => {
                            pushed = true;
                            "ok"
                        },
                        | _ => "",
                    }))
                },
                |args| {
                    if scenario == "gh_failure" {
                        return Ok(git::Output {
                            code: 1,
                            error: "not authenticated".into(),
                            ..reply("")
                        });
                    }
                    if args[1] == "create" {
                        created = true;
                    }
                    Ok(reply(if scenario == "existing" {
                        "[{\"url\":\"https://github.com/owner/repo/pull/9\"}]"
                    } else {
                        "[]"
                    }))
                },
            );
            if scenario == "existing" {
                assert_eq!(result.unwrap().url.as_deref(), Some("https://github.com/owner/repo/pull/9"));
                assert!(pushed);
                assert!(!created);
            } else {
                assert!(result.is_err(), "{scenario}");
                assert!(!pushed && !created, "{scenario}");
            }
        }
    }

    #[test]
    fn rejects_ambiguous_or_credential_bearing_remotes() {
        assert_eq!(repository("git@github.com:owner/repo.git").unwrap(), "owner/repo");
        for url in [
            "https://token@github.com/a/b",
            "https://github.com/a/b/c",
            "https://evil.test/a/b",
            "https://github.com/../b",
        ] {
            assert!(repository(url).is_err());
        }
    }
    #[test]
    fn confirms_push_and_writes_exact_body_without_shell_interpolation() {
        let version = GitVersion {
            branch: None,
            head: Some("head".into()),
            index: "index".into(),
            worktree: "tree".into(),
        };
        let command = GitCommand::CreatePr {
            remote: "origin".into(),
            base: "main".into(),
            title: "Fix 한글".into(),
            body: "line one\n`literal` $(literal)".into(),
            draft: true,
            push: true,
            expected: version.clone(),
            expected_base: "base".into(),
            expected_branch: "feature/test".into(),
            expected_repository: "owner/repo".into(),
        };
        let mut pushed = false;
        let mut created = false;
        let result = perform(
            &command,
            &version,
            |args| {
                let result = match args[0] {
                    | "remote" if args.len() == 1 => "origin",
                    | "remote" => "https://github.com/owner/repo.git",
                    | "symbolic-ref" => "feature/test",
                    | "rev-parse" => "base",
                    | "log" => "head fix",
                    | "diff" => "+change",
                    | "push" => {
                        assert_eq!(args.last(), Some(&"head:refs/heads/feature/test"));
                        pushed = true;
                        "ok"
                    },
                    | _ => "",
                };
                Ok(git::Output {
                    code: 0,
                    bytes: result.as_bytes().to_vec(),
                    error: String::new(),
                    truncated: false,
                })
            },
            |args| {
                let result = if args[1] == "list" {
                    "[]"
                } else {
                    created = true;
                    let path = args[args.iter().position(|s| *s == "--body-file").unwrap() + 1];
                    assert_eq!(std::fs::read_to_string(path).unwrap(), "line one\n`literal` $(literal)");
                    assert!(args.contains(&"--draft"));
                    "https://github.com/owner/repo/pull/7"
                };
                Ok(git::Output {
                    code: 0,
                    bytes: result.as_bytes().to_vec(),
                    error: String::new(),
                    truncated: false,
                })
            },
        )
        .unwrap();
        assert!(pushed && created);
        assert_eq!(result.url.as_deref(), Some("https://github.com/owner/repo/pull/7"));
    }
}
