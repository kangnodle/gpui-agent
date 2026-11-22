use super::*;
use gpui_agent_domain::{Checkpoint,
                        GitCommand,
                        GitOverview,
                        GitVersion};
use std::{collections::HashSet,
          ffi::OsStr,
          io::Write};

fn text(path: &Path, hooks: &Path, args: &[&str]) -> ApplicationResult<String> {
    let out = git::run(path, hooks, args)?.require()?;
    if out.truncated {
        return Err(failure("Git output exceeds the safe limit"));
    }
    Ok(out.text().trim().to_owned())
}
fn paths(path: &Path, hooks: &Path, args: &[&str]) -> ApplicationResult<Vec<String>> {
    let out = git::run(path, hooks, args)?.require()?;
    if out.truncated {
        return Err(failure("Too many paths to checkpoint safely"));
    }
    out.bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8(s.to_vec()).map_err(|_| invalid("Git paths must be UTF-8")))
        .collect()
}
fn safe_path(root: &Path, relative: &str) -> ApplicationResult<PathBuf> {
    let mut result = root.to_path_buf();
    for part in Path::new(relative).components() {
        let std::path::Component::Normal(part) = part else {
            return Err(invalid("Unsafe Git path"));
        };
        result.push(part);
        if let Ok(metadata) = std::fs::symlink_metadata(&result) {
            #[cfg(windows)]
            let link = {
                use std::os::windows::fs::MetadataExt;
                metadata.file_attributes() & 0x400 != 0
            };
            #[cfg(not(windows))]
            let link = metadata.file_type().is_symlink();
            if link {
                return Err(invalid("Checkpoints do not follow symbolic links or junctions"));
            }
        }
    }
    Ok(result)
}
fn tree_paths(path: &Path, hooks: &Path, tree: &str) -> ApplicationResult<Vec<String>> {
    let records = paths(path, hooks, &["ls-tree", "-r", "-z", tree])?;
    records
        .into_iter()
        .map(|record| {
            if !record.starts_with("100644 ") && !record.starts_with("100755 ") {
                return Err(invalid("Checkpoints support regular files; submodules and symlinks are unsupported"));
            }
            record.split_once('\t').map(|(_, p)| p.to_owned()).ok_or_else(|| failure("Invalid tree record"))
        })
        .collect()
}
fn snapshot(path: &Path, hooks: &Path) -> ApplicationResult<GitVersion> { snapshot_using_index(path, hooks, None) }
fn snapshot_using_index(path: &Path, hooks: &Path, index_file: Option<&Path>) -> ApplicationResult<GitVersion> {
    for feature in ["core.sparseCheckout", "core.splitIndex"] {
        let enabled = git::run(path, hooks, &["config", "--bool", feature])?;
        if enabled.code == 0 && enabled.text().trim() == "true" {
            return Err(invalid("Disable sparse checkout and split-index before using checkpoints"));
        }
    }
    let split = text(path, hooks, &["rev-parse", "--shared-index-path"])?;
    if !split.is_empty() {
        return Err(invalid("Disable sparse checkout and split-index before using checkpoints"));
    }

    for entry in paths(path, hooks, &["ls-files", "-v", "-z"])? {
        if entry.starts_with('S') || entry.as_bytes().first().is_some_and(u8::is_ascii_lowercase) {
            return Err(invalid("Disable sparse checkout, skip-worktree and assume-unchanged before using checkpoints"));
        }
    }
    for name in paths(path, hooks, &["ls-files", "--cached", "--others", "--exclude-standard", "-z"])? {
        safe_path(path, &name)?;
    }
    let index = match index_file {
        | Some(index) => git::run_env(path, hooks, &["write-tree"], &[("GIT_INDEX_FILE", index.as_os_str())])?
            .require()?
            .text()
            .trim()
            .to_owned(),
        | None => text(path, hooks, &["write-tree"])?,
    };
    tree_paths(path, hooks, &index)?;
    let temp = tempfile::tempdir().map_err(|e| failure(e.to_string()))?;
    let temp_index = temp.path().join("index");
    let env = [("GIT_INDEX_FILE", temp_index.as_os_str())];
    git::run_env(path, hooks, &["read-tree", &index], &env)?.require()?;
    git::run_env(path, hooks, &["add", "-A", "--", "."], &env)?.require()?;
    let worktree = git::run_env(path, hooks, &["write-tree"], &env)?.require()?.text().trim().to_owned();
    tree_paths(path, hooks, &worktree)?;
    let head = git::run(path, hooks, &["rev-parse", "--verify", "HEAD"])?;
    Ok(GitVersion {
        branch: git::branch(path, hooks),
        head: (head.code == 0).then(|| head.text().trim().to_owned()),
        index,
        worktree,
    })
}
fn save(store: &Projects, db: &Connection, id: i64, path: &Path, label: &str, version: &GitVersion) -> ApplicationResult<String> {
    let key = new_token().map_err(|e| failure(e.to_string()))?;
    let env = [
        ("GIT_AUTHOR_NAME", OsStr::new("Gpui Agent")),
        ("GIT_AUTHOR_EMAIL", OsStr::new("checkpoint@localhost")),
        ("GIT_COMMITTER_NAME", OsStr::new("Gpui Agent")),
        ("GIT_COMMITTER_EMAIL", OsStr::new("checkpoint@localhost")),
    ];
    let index = git::run_env(
        path,
        &store.hooks,
        &["-c", "commit.gpgSign=false", "commit-tree", &version.index, "-m", "Checkpoint index"],
        &env,
    )?
    .require()?
    .text()
    .trim()
    .to_owned();
    let mut args = vec!["-c", "commit.gpgSign=false", "commit-tree", &version.worktree, "-p", &index, "-m", label];
    if let Some(head) = &version.head {
        args.extend(["-p", head]);
    }
    let commit = git::run_env(path, &store.hooks, &args, &env)?.require()?.text().trim().to_owned();
    text(path, &store.hooks, &["update-ref", &format!("refs/gpui-agent/checkpoints/{key}"), &commit])?;
    db.execute(
        "INSERT INTO checkpoints(id,workspace,label,head,snapshot,index_tree) VALUES(?1,?2,?3,?4,?5,?6)",
        params![key, id, label, version.head, commit, version.index],
    )
    .app()?;
    Ok(key)
}
fn target(db: &Connection, id: i64, key: &str) -> ApplicationResult<(Option<String>, String, String)> {
    db.query_row(
        "SELECT head,snapshot,index_tree FROM checkpoints WHERE id=?1 AND workspace=?2",
        params![key, id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
    .app()?
    .ok_or_else(missing)
}
struct IndexLock(PathBuf);
impl Drop for IndexLock {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

pub(super) fn execute(store: &Projects, id: i64, command: &GitCommand) -> ApplicationResult<GitOverview> {
    command.validate()?;
    let state = match store.state.lock() {
        | Ok(state) => state,
        | Err(error) => return Err(failure(format!("Project database lock poisoned: {error}"))),
    };
    let workspace = store.checked_workspace(&state.db, id)?;
    let intent_only = matches!(command, GitCommand::Inspect | GitCommand::ProposeCommit { .. });
    if !intent_only && store.active_application(Some(id)).is_some_and(|a| a.is_busy()) {
        return Err(conflict("Stop the active workspace operation before changing Git state"));
    }
    let _locks = if intent_only { Vec::new() } else { store.profile_locks()? };
    let path = Path::new(&workspace.path);
    let top = text(path, &store.hooks, &["rev-parse", "--show-toplevel"])?;
    let home = std::fs::canonicalize(&store.settings.home).map_err(|e| failure(e.to_string()))?;
    let root = std::fs::canonicalize(path).map_err(|e| failure(e.to_string()))?;
    if home.starts_with(&root) {
        return Err(invalid(
            "Application home must be outside the repository to keep credentials out of checkpoints",
        ));
    }
    if canonical(Path::new(&top))? != workspace.path {
        return Err(invalid("Select the repository root to use checkpoints and commits"));
    }
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
        "BISECT_LOG",
    ] {
        let location = text(path, &store.hooks, &["rev-parse", "--path-format=absolute", "--git-path", marker])?;
        if Path::new(&location).exists() {
            return Err(conflict("Finish the merge, rebase, cherry-pick or bisect before using checkpoints and commits"));
        }
    }
    let mut version = snapshot(path, &store.hooks)?;
    let mut pr = None;
    let mut backup = None;
    let mut commit = None;
    let mut diff_args = vec!["diff", "--no-ext-diff", "--no-textconv", "--no-color", "--cached"];
    let mut restore_tree = None;
    let mut restore_index = None;
    match command {
        | GitCommand::Inspect => {},
        | GitCommand::ProposeCommit {
            message,
            paths,
        } => {
            selected_paths(path, &store.hooks, paths, &version.worktree)?;
            let proposal = gpui_agent_domain::CommitProposal {
                message: message.clone(),
                paths: paths.clone(),
                expected: version.clone(),
            };
            let proposal = serde_json::to_string(&proposal).map_err(|e| failure(e.to_string()))?;
            state
                .db
                .execute(
                    "INSERT INTO commit_proposals(workspace,proposal) VALUES(?1,?2) ON CONFLICT(workspace) DO UPDATE SET proposal=excluded.proposal",
                    params![id, proposal],
                )
                .app()?;
        },
        | GitCommand::CommitSelected {
            message,
            paths,
            expected,
        } => {
            if expected != &version {
                return Err(conflict("Files or staging changed since review; refresh before committing"));
            }
            let (selected, removed) = selected_paths(path, &store.hooks, paths, &version.worktree)?;
            commit = Some(commit_selected(path, &store.hooks, message, &selected, &removed, &version)?);
            state.db.execute("DELETE FROM commit_proposals WHERE workspace=?1", [id]).app()?;
            version = snapshot(path, &store.hooks)?;
        },
        | GitCommand::PreviewPr {
            ..
        }
        | GitCommand::CreatePr {
            ..
        } => {
            if !git::changes(path, &store.hooks, false)?.is_empty() {
                return Err(conflict("Commit or stash changed and untracked files before creating a PR"));
            }
            pr = Some(super::pull_requests::run(path, &store.hooks, command, &version)?);
        },
        | GitCommand::Stage {
            path: file,
            staged,
            expected,
        } => {
            if expected != &version {
                return Err(conflict("Files or staging changed; refresh before staging"));
            }
            let files = git::changes(path, &store.hooks, false)?;
            let file = files.iter().find(|f| f.path == *file).ok_or_else(missing)?;
            let empty = text(path, &store.hooks, &["mktree"])?;
            let source = version.head.as_deref().unwrap_or(&empty);
            let mut args = if *staged {
                vec!["add", "-A", "--", &file.path]
            } else {
                vec!["restore", "--staged", "--source", source, "--", &file.path]
            };
            if let Some(original) = &file.original_path {
                args.push(original);
            }
            text(path, &store.hooks, &args)?;
            version = snapshot(path, &store.hooks)?;
        },
        | GitCommand::Checkpoint {
            label,
        } => {
            save(store, &state.db, id, path, label.trim(), &version)?;
        },
        | GitCommand::PreviewRestore {
            checkpoint,
        }
        | GitCommand::Restore {
            checkpoint, ..
        } => {
            let (head, tree, index) = target(&state.db, id, checkpoint)?;
            if head != version.head {
                return Err(conflict("Checkpoint belongs to a different HEAD; switch back before restoring"));
            }
            if let GitCommand::Restore {
                expected, ..
            } = command
            {
                if expected != &version {
                    return Err(conflict("Files or staging changed since preview; preview again"));
                }
                let current: HashSet<_> = tree_paths(path, &store.hooks, &version.worktree)?.into_iter().collect();
                for name in tree_paths(path, &store.hooks, &tree)? {
                    let absolute = safe_path(path, &name)?;
                    if absolute.is_dir() || (absolute.exists() && !current.contains(&name)) {
                        return Err(conflict("An ignored file or directory blocks restoration; move it aside first"));
                    }
                    let mut parent = absolute.parent();
                    while let Some(ancestor) = parent {
                        if ancestor == path {
                            break;
                        }
                        if ancestor.is_file() {
                            return Err(conflict("A file blocks a checkpoint directory; move it aside first"));
                        }
                        parent = ancestor.parent();
                    }
                }
                let index_path = PathBuf::from(text(path, &store.hooks, &["rev-parse", "--path-format=absolute", "--git-path", "index"])?);
                let lock_path = index_path.with_extension("lock");
                let mut locked = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&lock_path)
                    .map_err(|_| conflict("Git index is locked by another operation"))?;
                let lock = IndexLock(lock_path);
                // Only the lock owner may remove it.
                let key = save(store, &state.db, id, path, "Before restore", &version)?;
                let temp = tempfile::tempdir().map_err(|e| failure(e.to_string()))?;
                let temp_index = temp.path().join("index");
                let env = [("GIT_INDEX_FILE", temp_index.as_os_str())];
                let restored = (|| {
                    git::run_env(path, &store.hooks, &["read-tree", &version.worktree], &env)?.require()?;
                    git::run_env(path, &store.hooks, &["read-tree", "--reset", "-u", &tree], &env)?.require()?;
                    git::run_env(path, &store.hooks, &["read-tree", &index], &env)?.require()?;
                    locked
                        .write_all(&std::fs::read(&temp_index).map_err(|e| failure(e.to_string()))?)
                        .map_err(|e| failure(e.to_string()))?;
                    locked.sync_all().map_err(|e| failure(e.to_string()))?;
                    drop(locked);
                    std::fs::rename(&lock.0, &index_path).map_err(|e| failure(e.to_string()))?;
                    Ok::<_, ApplicationError>(())
                })();
                if let Err(error) = restored {
                    return Err(failure(format!("Restore failed: {}; recovery checkpoint: {key}", error.1)));
                }
                backup = Some(key);
                version = snapshot(path, &store.hooks)?;
            } else {
                restore_tree = Some(tree);
                restore_index = Some(index);
            }
        },
        | GitCommand::Commit {
            message,
            expected,
        } => {
            if expected != &version {
                return Err(conflict("Files or staging changed since preview; refresh before committing"));
            }
            if git::branch(path, &store.hooks).is_none() {
                return Err(conflict("Select a branch before committing"));
            }
            text(
                path,
                &store.hooks,
                &["-c", "commit.gpgSign=false", "commit", "--no-gpg-sign", "-m", message.trim()],
            )?;
            commit = Some(text(path, &store.hooks, &["rev-parse", "HEAD"])?);
            version = snapshot(path, &store.hooks)?;
        },
    }
    if let Some(tree) = &restore_tree {
        diff_args = vec!["diff", "--no-ext-diff", "--no-textconv", "--no-color", &version.worktree, tree, "--"];
    }
    let out = git::run(path, &store.hooks, &diff_args)?.require()?;
    let mut diff = out.text();
    let mut truncated = out.truncated;
    if let Some(index) = restore_index {
        let staged = git::run(
            path,
            &store.hooks,
            &["diff", "--no-ext-diff", "--no-textconv", "--no-color", &version.index, &index, "--"],
        )?
        .require()?;
        diff = format!("Working tree → checkpoint\n{diff}\nIndex → checkpoint staging\n{}", staged.text());
        truncated |= staged.truncated;
    }
    let mut query = state
        .db
        .prepare("SELECT id,label,created,head FROM checkpoints WHERE workspace=?1 ORDER BY created DESC,rowid DESC")
        .app()?;
    let checkpoints = query
        .query_map([id], |r| {
            Ok(Checkpoint {
                id: r.get(0)?,
                label: r.get(1)?,
                created_at: r.get(2)?,
                head: r.get(3)?,
            })
        })
        .app()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .app()?;
    Ok(GitOverview {
        workspace_id: id,
        version,
        checkpoints,
        files: git::changes(path, &store.hooks, false)?,
        diff,
        truncated,
        backup,
        commit,
        pr,
        proposal: state
            .db
            .query_row("SELECT proposal FROM commit_proposals WHERE workspace=?1", [id], |row| row.get::<_, String>(0))
            .optional()
            .app()?
            .map(|value| serde_json::from_str(&value).map_err(|e| failure(e.to_string())))
            .transpose()?,
    })
}

fn selected_paths(path: &Path, hooks: &Path, selected: &[String], tree: &str) -> ApplicationResult<(Vec<String>, Vec<String>)> {
    let files = git::changes(path, hooks, false)?;
    let existing = tree_paths(path, hooks, tree)?;
    let mut paths = Vec::new();
    let mut removed = Vec::new();
    for name in selected {
        let file = files
            .iter()
            .find(|file| file.path == *name)
            .ok_or_else(|| invalid("Select exact changed file paths"))?;
        safe_path(path, name)?;
        let target = if existing.contains(name) { &mut paths } else { &mut removed };
        if !target.contains(name) {
            target.push(name.clone());
        }
        if let Some(original) = &file.original_path {
            safe_path(path, original)?;
            if !selected.contains(original) && !removed.contains(original) {
                removed.push(original.clone());
            }
        }
    }
    Ok((paths, removed))
}

fn commit_selected(path: &Path, hooks: &Path, message: &str, selected: &[String], removed: &[String], expected: &GitVersion) -> ApplicationResult<String> {
    if git::branch(path, hooks).is_none() {
        return Err(conflict("Select a branch before committing"));
    }
    let temp = tempfile::tempdir().map_err(|e| failure(e.to_string()))?;
    let selected_index = temp.path().join("selected");
    let preserved_index = temp.path().join("preserved");
    let env = [("GIT_INDEX_FILE", selected_index.as_os_str())];
    let preserved = [("GIT_INDEX_FILE", preserved_index.as_os_str())];
    let empty = text(path, hooks, &["mktree"])?;
    git::run_env(path, hooks, &["read-tree", expected.head.as_deref().unwrap_or(&empty)], &env)?.require()?;
    let mut add = vec!["restore", "--staged", "--source", &expected.worktree, "--"];
    add.extend(selected.iter().map(String::as_str));
    if !selected.is_empty() {
        git::run_env(path, hooks, &add, &env)?.require()?;
    }
    let mut remove = vec!["update-index", "--force-remove", "--"];
    remove.extend(removed.iter().map(String::as_str));
    if !removed.is_empty() {
        git::run_env(path, hooks, &remove, &env)?.require()?;
    }
    let tree = git::run_env(path, hooks, &["write-tree"], &env)?.require()?.text().trim().to_owned();
    let index_path = PathBuf::from(text(path, hooks, &["rev-parse", "--path-format=absolute", "--git-path", "index"])?);
    let lock_path = index_path.with_extension("lock");
    let mut locked = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .map_err(|_| conflict("Git index is locked by another operation"))?;
    let lock = IndexLock(lock_path);
    let verification_index = temp.path().join("verification");
    if index_path.exists() {
        std::fs::copy(&index_path, &verification_index).map_err(|e| failure(e.to_string()))?;
    } else {
        git::run_env(path, hooks, &["read-tree", &empty], &[("GIT_INDEX_FILE", verification_index.as_os_str())])?.require()?;
    }
    if &snapshot_using_index(path, hooks, Some(&verification_index))? != expected {
        return Err(conflict("Files or staging changed since review; refresh before committing"));
    }
    if index_path.exists() {
        std::fs::copy(&index_path, &preserved_index).map_err(|e| failure(e.to_string()))?;
    } else {
        git::run_env(path, hooks, &["read-tree", &empty], &preserved)?.require()?;
    }
    let mut restore = vec!["restore", "--staged", "--source", &tree, "--"];
    restore.extend(selected.iter().map(String::as_str));
    if !selected.is_empty() {
        git::run_env(path, hooks, &restore, &preserved)?.require()?;
    }
    if !removed.is_empty() {
        git::run_env(path, hooks, &remove, &preserved)?.require()?;
    }
    locked
        .write_all(&std::fs::read(&preserved_index).map_err(|e| failure(e.to_string()))?)
        .map_err(|e| failure(e.to_string()))?;
    locked.sync_all().map_err(|e| failure(e.to_string()))?;
    let commit = publish_commit(path, hooks, message, &selected_index, expected)?;
    drop(locked);
    std::fs::rename(&lock.0, &index_path).map_err(|e| failure(format!("Commit completed but index update failed; inspect Git before retrying: {e}")))?;
    Ok(commit)
}

pub(super) fn publish_commit(path: &Path, hooks: &Path, message: &str, index: &Path, expected: &GitVersion) -> ApplicationResult<String> {
    let branch = expected.branch.as_ref().ok_or_else(|| conflict("Select a branch before committing"))?;
    let tree = git::run_env(path, hooks, &["write-tree"], &[("GIT_INDEX_FILE", index.as_os_str())])?
        .require()?
        .text()
        .trim()
        .to_owned();
    let base_tree = match &expected.head {
        | Some(head) => text(path, hooks, &["rev-parse", &format!("{head}^{{tree}}")])?,
        | None => text(path, hooks, &["mktree"])?,
    };
    if tree == base_tree {
        return Err(invalid("Selected files contain no changes to commit"));
    }
    let mut args = vec!["-c", "commit.gpgSign=false", "commit-tree", &tree, "-m", message];
    if let Some(head) = &expected.head {
        args.extend(["-p", head]);
    }
    let commit = text(path, hooks, &args)?;
    // shortcut: forced symbolic-ref changes can race this check; use atomic
    // symref verification when Git supports updating its referent in the same
    // transaction.
    if git::branch(path, hooks).as_ref() != Some(branch) {
        return Err(conflict("Branch changed since review; refresh before committing"));
    }
    let branch = format!("refs/heads/{branch}");
    let absent = "0".repeat(commit.len());
    let old = expected.head.as_deref().unwrap_or(&absent);
    let published = git::run(
        path,
        hooks,
        &["update-ref", "--no-deref", "-m", "commit: approved selection", &branch, &commit, old],
    )?;
    if published.code != 0 {
        return Err(conflict(format!(
            "Git could not atomically publish the reviewed branch; refresh before retrying: {}",
            published.error.trim()
        )));
    }
    Ok(commit)
}
