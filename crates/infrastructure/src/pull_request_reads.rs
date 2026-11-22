use super::*;
use gpui_agent_domain::*;
use serde_json::Value;
use std::process::{Command,
                   Stdio};

impl Projects {
    pub(super) fn read_pr(&self, id: i64, target: &PullRequestTarget, kind: PullRequestReadKind) -> ApplicationResult<PullRequestData> {
        target.validate()?;
        let path = {
            let state = self.state.lock().map_err(|_| failure("Project database lock poisoned"))?;
            PathBuf::from(self.checked_workspace(&state.db, id)?.path)
        };
        let remotes = git::run(&path, &self.hooks, &["remote"])?.require()?;
        if remotes.truncated {
            return Err(failure("Too many Git remotes"));
        }
        let mut matches = false;
        for remote in remotes.text().lines() {
            let out = git::run(&path, &self.hooks, &["remote", "get-url", "--all", remote])?.require()?;
            if out.truncated {
                return Err(failure("Git remote output too large"));
            }
            matches |= out
                .text()
                .lines()
                .any(|url| super::pull_requests::repository(url).is_ok_and(|repo| repo.eq_ignore_ascii_case(&target.repository)));
        }
        if !matches {
            return Err(invalid("The PR repository must match a GitHub remote of this workspace"));
        }
        let number = target.number.to_string();
        let repository = format!("github.com/{}", target.repository);
        let read = || -> ApplicationResult<Value> {
            let raw = gh(
                &path,
                &[
                    "pr",
                    "view",
                    &number,
                    "--repo",
                    &repository,
                    "--json",
                    "number,url,title,body,headRefOid,baseRefOid,headRefName,baseRefName,state,isDraft,mergeable,statusCheckRollup,comments,reviews",
                ],
            )?;
            serde_json::from_str(&raw).map_err(|_| failure("GitHub returned invalid PR JSON"))
        };
        let value = read()?;
        let parsed = snapshot(&value, target)?;
        let description = string(&value, "body")?.to_owned();
        if description.len() > 65536 {
            return Err(failure("PR description exceeds 64 KiB"));
        }
        let mut data = PullRequestData {
            snapshot: parsed,
            description,
            diff: None,
            viewer: None,
        };
        if matches!(kind, PullRequestReadKind::Watch) {
            let viewer = gh(&path, &["api", "user", "--hostname", "github.com", "--jq", ".login"])?;
            if viewer.is_empty() || viewer.len() > 100 || !viewer.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_[].".contains(&b)) {
                return Err(failure("GitHub returned an invalid viewer"));
            }
            data.viewer = Some(viewer);
        }
        if matches!(kind, PullRequestReadKind::Guide) {
            let diff = gh(&path, &["pr", "diff", &number, "--repo", &repository, "--color", "never"])?;
            if diff.len() > 192 * 1024 {
                return Err(invalid("PR diff exceeds the 192 KiB guide limit"));
            }
            let after = snapshot(&read()?, target)?;
            if after.head_sha != data.snapshot.head_sha || after.base_sha != data.snapshot.base_sha || after.base_branch != data.snapshot.base_branch {
                return Err(conflict("PR changed while reading its diff; retry"));
            }
            data.diff = Some(diff);
        }
        Ok(data)
    }
}
fn gh(cwd: &Path, args: &[&str]) -> ApplicationResult<String> {
    let exe = std::env::var_os("gpui_agent_GH_EXECUTABLE").unwrap_or_else(|| if cfg!(windows) { "gh.exe".into() } else { "gh".into() });
    let mut command = Command::new(exe);
    command
        .current_dir(cwd)
        .args(args)
        .env("GH_HOST", "github.com")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_PAGER", "cat")
        .env_remove("GH_REPO")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let out = git::execute(command)?;
    if out.code != 0 {
        let error = out.error.to_ascii_lowercase();
        if error.contains("rate limit") || error.contains("http 429") {
            return Err(conflict("GitHub rate limit; polling delayed"));
        }
        return Err(failure("GitHub PR read failed; check gh authentication and repository access"));
    }
    if out.truncated {
        return Err(failure("GitHub PR output exceeds 2 MiB"));
    }
    Ok(out.text().trim().into())
}
fn string<'a>(value: &'a Value, key: &str) -> ApplicationResult<&'a str> {
    value[key].as_str().ok_or_else(|| failure(format!("Missing GitHub PR field: {key}")))
}
fn snapshot(value: &Value, target: &PullRequestTarget) -> ApplicationResult<PullRequestSnapshot> {
    if value["number"].as_u64() != Some(target.number as u64) || string(value, "url")?.to_ascii_lowercase() != target.url() {
        return Err(failure("GitHub returned a different PR"));
    }
    let head = string(value, "headRefOid")?;
    let base = string(value, "baseRefOid")?;
    if [head, base].iter().any(|sha| sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit())) {
        return Err(failure("GitHub returned an invalid commit SHA"));
    }
    let state = match string(value, "state")? {
        | "OPEN" => PullRequestState::Open,
        | "CLOSED" => PullRequestState::Closed,
        | "MERGED" => PullRequestState::Merged,
        | _ => return Err(failure("Unknown PR state")),
    };
    let contexts = array(value, "statusCheckRollup", 256)?;
    let checks = contexts
        .iter()
        .map(|check| {
            let name = check["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .or_else(|| check["context"].as_str())
                .ok_or_else(|| failure("Missing check name"))?;
            let status = check["state"]
                .as_str()
                .filter(|state| !state.is_empty())
                .or_else(|| check["status"].as_str())
                .unwrap_or("PENDING");
            let status = if status == "COMPLETED" {
                check["conclusion"].as_str().unwrap_or("UNKNOWN")
            } else {
                status
            };
            let state = match status {
                | "SUCCESS" | "NEUTRAL" | "SKIPPED" => "passed",
                | "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => "failed",
                | _ => "pending",
            };
            Ok(PullRequestCheck {
                name: name.chars().take(256).collect(),
                state: state.into(),
            })
        })
        .collect::<ApplicationResult<Vec<_>>>()?;
    let mut remarks = vec![];
    // shortcut: gh pr view omits inline review threads; add a paginated
    // activity reader when those must wake agents.
    for key in ["comments", "reviews"] {
        for remark in array(value, key, 200)? {
            if key == "reviews" && remark["state"] == "PENDING" {
                continue;
            }
            let id = string(remark, "id")?;
            let at = remark["updatedAt"]
                .as_str()
                .or_else(|| remark["submittedAt"].as_str())
                .or_else(|| remark["createdAt"].as_str())
                .ok_or_else(|| failure("Missing PR remark date"))?;
            if id.len() > 256 || at.len() > 64 {
                return Err(failure("PR remark metadata exceeds limits"));
            }
            remarks.push(PullRequestRemark {
                id: format!("{key}:{id}"),
                at: at.into(),
                author: remark["author"]["login"].as_str().unwrap_or("ghost").chars().take(100).collect(),
                text: remark["body"].as_str().unwrap_or("").chars().take(200).collect(),
            });
        }
    }
    Ok(PullRequestSnapshot {
        title: string(value, "title")?.chars().take(256).collect(),
        head_sha: head.into(),
        base_sha: base.into(),
        head_branch: string(value, "headRefName")?.into(),
        base_branch: string(value, "baseRefName")?.into(),
        state,
        draft: value["isDraft"].as_bool().ok_or_else(|| failure("Missing draft state"))?,
        conflicting: match value["mergeable"].as_str() {
            | Some("CONFLICTING") => Some(true),
            | Some("MERGEABLE") => Some(false),
            | _ => None,
        },
        checks,
        remarks,
    })
}
fn array<'a>(value: &'a Value, key: &str, limit: usize) -> ApplicationResult<&'a [Value]> {
    match value.get(key) {
        | Some(Value::Null) => Ok(&[]),
        | Some(Value::Array(items)) if items.len() <= limit => Ok(items),
        | _ => Err(failure(format!("Invalid or oversized PR {key}"))),
    }
}
