use super::*;
use std::process::Command;
fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args([
            "-c",
            "core.hooksPath=disabled-hooks",
            "-c",
            "user.name=Workspace Test",
            "-c",
            "user.email=workspace@example.invalid",
            "-C",
        ])
        .arg(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}
struct Fixture {
    _root: tempfile::TempDir,
    repo: PathBuf,
    settings: Settings,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("project 한글 space");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("tracked.txt"), "original\n").unwrap();
        std::fs::write(repo.join(".gitignore"), "ignored.txt\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "initial"]);
        let settings = Settings::discover(repo.clone(), Some(root.path().join("home"))).unwrap();
        Self {
            _root: root,
            repo,
            settings,
        }
    }

    fn open(&self) -> Projects { Projects::open(self.settings.clone(), self.settings.application().unwrap()).unwrap() }
}
#[test]
fn native_worktree_creation_protects_dirty_locked_main_and_external_directories() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = store.catalog().unwrap();
    let project = catalog.projects[0].id;
    assert!(store.remove_worktree(catalog.default_workspace_id.unwrap()).is_err());
    for branch in ["../escape", "--force", "bad branch", "@{-1}"] {
        assert!(store.create_worktree(project, branch, "HEAD").is_err(), "{branch}");
    }
    assert!(store.create_worktree(project, "missing-base", "--bad").is_err());
    let catalog = store.create_worktree(project, "feature/한글", "HEAD").unwrap();
    let tree = catalog.projects[0].workspaces.iter().find(|tree| tree.managed).unwrap().clone();
    let path = Path::new(&tree.path);
    assert!(path.join("tracked.txt").exists());
    assert_eq!(tree.branch.as_deref(), Some("feature/한글"));
    assert!(store.create_worktree(project, "feature/한글", "HEAD").is_err());
    for name in ["tracked.txt", "untracked.txt", "ignored.txt"] {
        std::fs::write(path.join(name), "keep me").unwrap();
        assert!(matches!(store.remove_worktree(tree.id), Err(ApplicationError(ErrorKind::Busy, _))));
        assert_eq!(std::fs::read_to_string(path.join(name)).unwrap(), "keep me");
        if name == "tracked.txt" {
            git(path, &["restore", "--", "tracked.txt"]);
        } else {
            std::fs::remove_file(path.join(name)).unwrap();
        }
    }
    git(&fixture.repo, &["worktree", "lock", "--reason", "protect", &tree.path]);
    assert!(
        store.catalog().unwrap().projects[0]
            .workspaces
            .iter()
            .find(|workspace| workspace.id == tree.id)
            .unwrap()
            .locked
    );
    assert!(store.remove_worktree(tree.id).is_err());
    git(&fixture.repo, &["worktree", "unlock", &tree.path]);
    drop(store);
    let store = fixture.open();
    assert!(
        store.catalog().unwrap().projects[0]
            .workspaces
            .iter()
            .any(|workspace| workspace.id == tree.id && workspace.managed)
    );
    let external = fixture._root.path().join("external");
    git(&fixture.repo, &["worktree", "add", "-b", "external", external.to_str().unwrap()]);
    let catalog = store.catalog().unwrap();
    let outside = catalog.projects[0]
        .workspaces
        .iter()
        .find(|workspace| workspace.branch.as_deref() == Some("external"))
        .unwrap();
    assert!(!outside.managed && !outside.main);
    assert!(store.remove_worktree(outside.id).is_err());
    assert!(external.exists());
    assert_eq!(
        store.register(external.to_str().unwrap()).unwrap().projects.len(),
        1,
        "Linked worktrees belong to their existing project"
    );
    for profile in [
        fixture.settings.home.join("codex"),
        fixture.settings.home.join("accounts").join("a".repeat(32)).join("codex"),
    ] {
        std::fs::create_dir_all(&profile).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(profile.join(".operation.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        assert!(store.remove_worktree(tree.id).is_err());
        drop(lock);
    }
    // Parallel tests fork `git` children that briefly inherit the released
    // lock's descriptor, so the flock can stay held for a moment after
    // `drop(lock)`.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match store.remove_worktree(tree.id) {
            | Err(ApplicationError(ErrorKind::Busy, _)) if std::time::Instant::now() < deadline => std::thread::yield_now(),
            | result => {
                result.unwrap();
                break;
            },
        }
    }
    assert!(!path.exists());
    git(&fixture.repo, &["show-ref", "--verify", "refs/heads/feature/한글"]);
}
#[test]
fn diffs_cover_staged_unstaged_renames_binary_untracked_and_literal_paths() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let id = store.default.unwrap();
    std::fs::write(fixture.repo.join("tracked.txt"), "staged line\n").unwrap();
    git(&fixture.repo, &["add", "tracked.txt"]);
    std::fs::write(fixture.repo.join("tracked.txt"), "unstaged line\n").unwrap();
    std::fs::write(fixture.repo.join("새 파일 space.txt"), "새 내용 🦀\n").unwrap();
    std::fs::write(fixture.repo.join("binary.bin"), [0, 1, 2, 3]).unwrap();
    let changes = store.changes(id).unwrap();
    assert_eq!(changes.files.len(), 3);
    let diff = store.diff(id, "tracked.txt").unwrap();
    assert!(diff.staged.contains("+staged line"));
    assert!(diff.unstaged.contains("+unstaged line"));
    let diff = store.diff(id, "새 파일 space.txt").unwrap();
    assert!(diff.untracked && diff.unstaged.contains("+새 내용 🦀"));
    assert!(store.diff(id, "binary.bin").unwrap().unstaged.contains("Binary files"));
    assert!(store.diff(id, "../outside").is_err());
    assert!(store.diff(id, "--output=stolen").is_err());
    git(&fixture.repo, &["restore", "--staged", "--worktree", "--", "tracked.txt"]);
    git(&fixture.repo, &["mv", "tracked.txt", "renamed space.txt"]);
    let changes = store.changes(id).unwrap();
    let renamed = changes.files.iter().find(|file| file.path == "renamed space.txt").unwrap();
    assert_eq!(renamed.original_path.as_deref(), Some("tracked.txt"));
    assert!(store.diff(id, "renamed space.txt").unwrap().staged.contains("rename from"));
    std::fs::write(fixture.repo.join("large.txt"), "large line\n".repeat(250_000)).unwrap();
    assert!(store.diff(id, "large.txt").unwrap().truncated);
}

#[test]
fn registered_subdirectory_shows_only_its_side_of_each_change() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let subdir = fixture.repo.join("notes");
    let sibling = fixture.repo.join("notes-old");
    std::fs::create_dir(&subdir).unwrap();
    std::fs::create_dir(&sibling).unwrap();
    for (dir, name) in [
        (&subdir, "draft.txt"),
        (&subdir, "outgoing.txt"),
        (&sibling, "incoming.txt"),
        (&sibling, "other.txt"),
    ] {
        std::fs::write(dir.join(name), name).unwrap();
    }
    git(&fixture.repo, &["add", "."]);
    git(&fixture.repo, &["commit", "-m", "notes"]);
    git(&fixture.repo, &["mv", "notes/draft.txt", "notes/final.txt"]);
    git(&fixture.repo, &["mv", "notes/outgoing.txt", "notes-old/moved.txt"]);
    git(&fixture.repo, &["mv", "notes-old/incoming.txt", "notes/received.txt"]);
    std::fs::write(sibling.join("other.txt"), "sibling edit\n").unwrap();
    std::fs::write(subdir.join("[todo].txt"), "new note\n").unwrap();
    let id = store.register(subdir.to_str().unwrap()).unwrap().registered_workspace_id.unwrap();
    let changes = store.changes(id).unwrap();
    let paths: std::collections::BTreeSet<_> = changes.files.iter().map(|file| (file.path.as_str(), file.original_path.as_deref())).collect();
    assert_eq!(
        paths,
        [
            ("notes/[todo].txt", None),
            ("notes/final.txt", Some("notes/draft.txt")),
            ("notes/outgoing.txt", None),
            ("notes/received.txt", None),
        ]
        .into()
    );
    assert!(store.diff(id, "notes/final.txt").unwrap().staged.contains("rename from notes/draft.txt"));
    for file in ["notes/outgoing.txt", "notes/received.txt"] {
        let diff = store.diff(id, file).unwrap();
        assert!(diff.staged.contains(file));
        assert!(!diff.staged.contains("notes-old/"));
    }
    assert!(store.diff(id, "notes/[todo].txt").unwrap().unstaged.contains("+new note"));
    assert!(store.diff(id, "notes-old/other.txt").is_err());
}
#[test]
fn canonical_project_registration_restores_ids_and_supports_plain_folders() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let initial = store.catalog().unwrap();
    let registered = store.register(fixture.repo.join(".").to_str().unwrap()).unwrap();
    assert_eq!(registered.projects.len(), 1);
    assert_eq!(registered.registered_workspace_id, initial.default_workspace_id);
    let plain = fixture._root.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    let catalog = store.register(plain.to_str().unwrap()).unwrap();
    let plain = catalog.projects.iter().find(|project| !project.git).unwrap();
    assert_eq!(catalog.registered_workspace_id, Some(plain.workspaces[0].id));
    assert_eq!(catalog.default_workspace_id, initial.default_workspace_id);
    let id = plain.id;
    assert!(store.create_worktree(id, "feature", "HEAD").is_err());
    assert!(store.changes(plain.workspaces[0].id).is_err());
    drop(store);
    let store = fixture.open();
    let catalog = store.catalog().unwrap();
    assert!(catalog.registered_workspace_id.is_none());
    assert_eq!(catalog.default_workspace_id, initial.default_workspace_id);
    assert!(catalog.projects.iter().any(|project| project.id == id));
}

#[test]
fn forgetting_projects_preserves_worktrees_history_and_ids_across_restart() {
    use gpui_agent_application::ports::ConversationRepository;
    use gpui_agent_domain::{Completion,
                            Event,
                            Prompt,
                            Provider};
    let fixture = Fixture::new();
    let store = fixture.open();
    let project = store.catalog().unwrap().projects[0].id;
    let catalog = store.create_worktree(project, "keep-worktree", "HEAD").unwrap();
    let tree = catalog.projects[0].workspaces.iter().find(|workspace| workspace.managed).unwrap().clone();
    std::fs::write(Path::new(&tree.path).join("keep.txt"), "keep this file").unwrap();
    let history = crate::History::open(&fixture.settings.home.join("history.sqlite3"), &fixture.repo).unwrap();
    let turn = history
        .begin(
            Provider::Claude,
            None,
            &mut Prompt {
                text: "Keep this conversation".into(),
                ..Default::default()
            },
        )
        .unwrap();
    history
        .event(
            turn,
            &Event::Text {
                delta: "Saved answer".into(),
            },
        )
        .unwrap();
    history
        .finish(
            turn,
            Some(&Completion {
                session_id: "keep-session".into(),
                text: "Saved answer".into(),
            }),
        )
        .unwrap();
    let plain = fixture._root.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    let catalog = store.register(plain.to_str().unwrap()).unwrap();
    let plain_id = catalog.registered_workspace_id.unwrap();
    let plain_project = catalog.projects.iter().find(|p| p.id != project).unwrap().id;
    let catalog = store.forget_project(project).unwrap();
    assert_eq!(catalog.projects.len(), 1);
    assert_eq!(catalog.default_workspace_id, Some(plain_id));
    assert_eq!(std::fs::read_to_string(Path::new(&tree.path).join("keep.txt")).unwrap(), "keep this file");
    assert_eq!(std::fs::read_to_string(fixture.repo.join("tracked.txt")).unwrap(), "original\n");
    assert!(store.forget_project(999999).is_err());
    let catalog = store.forget_project(plain_project).unwrap();
    assert!(catalog.projects.is_empty() && catalog.default_workspace_id.is_none());
    drop(store);
    let store = fixture.open();
    assert!(
        store.catalog().unwrap().projects.is_empty(),
        "Implicit startup registration must not restore hidden projects"
    );
    let restored = store.register(fixture.repo.to_str().unwrap()).unwrap();
    assert_eq!(restored.projects[0].id, project);
    assert!(
        restored.projects[0]
            .workspaces
            .iter()
            .any(|workspace| workspace.id == tree.id && workspace.managed)
    );
    assert_eq!(store.register(plain.to_str().unwrap()).unwrap().registered_workspace_id, Some(plain_id));
    let saved = history.load(Provider::Claude, turn.conversation_id).unwrap().unwrap();
    assert_eq!(saved.session_id.as_deref(), Some("keep-session"));
    assert_eq!(saved.messages[1].text, "Saved answer");
}

#[tokio::test]
async fn forgetting_projects_checks_all_workspaces_before_changing_the_list() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let id = store.catalog().unwrap().projects[0].id;
    let service = store.service();
    let result = service
        .forget_project(id, |ids| {
            assert_eq!(ids.len(), 1);
            Err(conflict("Project has queued work"))
        })
        .await;
    assert!(matches!(result, Err(ApplicationError(ErrorKind::Busy, _))));
    assert_eq!(service.catalog().await.unwrap().projects.len(), 1);
}
fn git_text(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(cwd).args(args).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}
#[test]
fn checkpoints_restore_staging_binary_untracked_and_deleted_files_with_recovery() {
    use gpui_agent_domain::GitCommand::*;
    let fixture = Fixture::new();
    let store = fixture.open();
    let id = store.default.unwrap();
    std::fs::write(fixture.repo.join("tracked.txt"), "staged A\n").unwrap();
    git(&fixture.repo, &["add", "tracked.txt"]);
    std::fs::write(fixture.repo.join("tracked.txt"), "working A\n").unwrap();
    std::fs::write(fixture.repo.join("새 파일.bin"), [0, 255, 1, 3]).unwrap();
    std::fs::write(fixture.repo.join("ignored.txt"), "keep ignored").unwrap();
    let original_status = git_text(&fixture.repo, &["status", "--porcelain=v1", "-z"]);
    let original_index = git_text(&fixture.repo, &["write-tree"]);
    let saved = store
        .git(
            id,
            &Checkpoint {
                label: "A 한글".into(),
            },
        )
        .unwrap();
    let key = saved.checkpoints[0].id.clone();
    assert_eq!(git_text(&fixture.repo, &["status", "--porcelain=v1", "-z"]), original_status);
    assert_eq!(git_text(&fixture.repo, &["write-tree"]), original_index);
    std::fs::write(fixture.repo.join("tracked.txt"), "staged B\n").unwrap();
    git(&fixture.repo, &["add", "tracked.txt"]);
    std::fs::write(fixture.repo.join("tracked.txt"), "working B\n").unwrap();
    std::fs::remove_file(fixture.repo.join("새 파일.bin")).unwrap();
    std::fs::remove_file(fixture.repo.join(".gitignore")).unwrap();
    // Keep the ignore rules in force while testing a tracked deletion.
    std::fs::write(fixture.repo.join(".git/info/exclude"), "ignored.txt\n").unwrap();
    std::fs::write(fixture.repo.join("later.txt"), "recover me").unwrap();
    assert!(
        store
            .git(
                id,
                &Restore {
                    checkpoint: key.clone(),
                    expected: saved.version
                }
            )
            .is_err()
    );
    let preview = store
        .git(
            id,
            &PreviewRestore {
                checkpoint: key.clone(),
            },
        )
        .unwrap();
    assert!(preview.diff.contains("working A"));
    let restored = store
        .git(
            id,
            &Restore {
                checkpoint: key.clone(),
                expected: preview.version,
            },
        )
        .unwrap();
    assert_eq!(std::fs::read_to_string(fixture.repo.join("tracked.txt")).unwrap(), "working A\n");
    assert_eq!(git_text(&fixture.repo, &["show", ":tracked.txt"]), "staged A\n");
    assert_eq!(std::fs::read(fixture.repo.join("새 파일.bin")).unwrap(), [0, 255, 1, 3]);
    assert!(!fixture.repo.join("later.txt").exists());
    assert_eq!(std::fs::read_to_string(fixture.repo.join("ignored.txt")).unwrap(), "keep ignored");
    assert_eq!(git_text(&fixture.repo, &["status", "--porcelain=v1", "-z"]), original_status);
    let backup = restored.backup.unwrap();
    let recovered = store
        .git(
            id,
            &Restore {
                checkpoint: backup,
                expected: restored.version,
            },
        )
        .unwrap();
    assert!(recovered.backup.is_some());
    assert_eq!(std::fs::read_to_string(fixture.repo.join("tracked.txt")).unwrap(), "working B\n");
    assert_eq!(git_text(&fixture.repo, &["show", ":tracked.txt"]), "staged B\n");
    assert_eq!(std::fs::read_to_string(fixture.repo.join("later.txt")).unwrap(), "recover me");
    assert!(!fixture.repo.join(".gitignore").exists());
    drop(store);
    assert!(fixture.open().git(id, &Inspect).unwrap().checkpoints.iter().any(|c| c.id == key));
}
#[test]
fn checkpoint_restore_refuses_other_head_ignored_collisions_and_foreign_index_locks() {
    use gpui_agent_domain::GitCommand::*;
    let fixture = Fixture::new();
    let store = fixture.open();
    let id = store.default.unwrap();
    std::fs::write(fixture.repo.join("saved.txt"), "snapshot").unwrap();
    let saved = store
        .git(
            id,
            &Checkpoint {
                label: "safe".into(),
            },
        )
        .unwrap();
    let key = saved.checkpoints[0].id.clone();
    std::fs::remove_file(fixture.repo.join("saved.txt")).unwrap();
    std::fs::write(fixture.repo.join(".git/info/exclude"), "saved.txt\n").unwrap();
    std::fs::write(fixture.repo.join("saved.txt"), "ignored content").unwrap();
    let preview = store
        .git(
            id,
            &PreviewRestore {
                checkpoint: key.clone(),
            },
        )
        .unwrap();
    assert!(
        store
            .git(
                id,
                &Restore {
                    checkpoint: key.clone(),
                    expected: preview.version
                }
            )
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(fixture.repo.join("saved.txt")).unwrap(), "ignored content");
    std::fs::remove_file(fixture.repo.join("saved.txt")).unwrap();
    let preview = store.git(id, &Inspect).unwrap();
    std::fs::write(fixture.repo.join(".git/index.lock"), "foreign owner").unwrap();
    assert!(
        store
            .git(
                id,
                &Restore {
                    checkpoint: key.clone(),
                    expected: preview.version
                }
            )
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(fixture.repo.join(".git/index.lock")).unwrap(), "foreign owner");
    std::fs::remove_file(fixture.repo.join(".git/index.lock")).unwrap();
    git(&fixture.repo, &["commit", "--allow-empty", "-m", "another HEAD"]);
    assert!(
        store
            .git(
                id,
                &PreviewRestore {
                    checkpoint: key
                }
            )
            .is_err()
    );
    let mut other = fixture.open();
    other.default = Some(99999);
    assert!(other.git(99999, &Inspect).is_err());
}
#[test]
fn commits_include_only_staged_changes_and_refuse_stale_or_empty_requests() {
    use gpui_agent_domain::GitCommand::*;
    let fixture = Fixture::new();
    let store = fixture.open();
    let id = store.default.unwrap();
    git(&fixture.repo, &["config", "user.name", "Commit Test"]);
    git(&fixture.repo, &["config", "user.email", "test@example.invalid"]);
    std::fs::write(fixture.repo.join("tracked.txt"), "staged commit\n").unwrap();
    git(&fixture.repo, &["add", "tracked.txt"]);
    std::fs::write(fixture.repo.join("tracked.txt"), "unstaged keep\n").unwrap();
    std::fs::write(fixture.repo.join("new.txt"), "untracked keep").unwrap();
    let preview = store.git(id, &Inspect).unwrap();
    assert!(preview.diff.contains("staged commit"));
    assert!(!preview.diff.contains("unstaged keep"));
    let result = store
        .git(
            id,
            &Commit {
                message: "Test commit 한글".into(),
                expected: preview.version.clone(),
            },
        )
        .unwrap();
    assert!(result.commit.is_some());
    assert_eq!(git_text(&fixture.repo, &["show", "HEAD:tracked.txt"]), "staged commit\n");
    assert_eq!(std::fs::read_to_string(fixture.repo.join("tracked.txt")).unwrap(), "unstaged keep\n");
    assert_eq!(std::fs::read_to_string(fixture.repo.join("new.txt")).unwrap(), "untracked keep");
    assert!(
        store
            .git(
                id,
                &Commit {
                    message: "stale".into(),
                    expected: preview.version
                }
            )
            .is_err()
    );
    assert!(
        store
            .git(
                id,
                &Commit {
                    message: "empty".into(),
                    expected: result.version
                }
            )
            .is_err()
    );
}
#[test]
fn checkpoints_work_in_linked_and_unborn_worktrees_and_enforce_workspace_ownership() {
    use gpui_agent_domain::GitCommand::*;
    let fixture = Fixture::new();
    let store = fixture.open();
    let main = store.default.unwrap();
    let saved = store
        .git(
            main,
            &Checkpoint {
                label: "main".into(),
            },
        )
        .unwrap();
    let catalog = store.create_worktree(saved.workspace_id, "feature/checkpoint", "HEAD").unwrap();
    let tree = catalog.projects[0].workspaces.iter().find(|w| w.managed).unwrap();
    assert!(matches!(
        store.git(
            tree.id,
            &PreviewRestore {
                checkpoint: saved.checkpoints[0].id.clone()
            }
        ),
        Err(ApplicationError(ErrorKind::NotFound, _))
    ));
    std::fs::write(Path::new(&tree.path).join("tracked.txt"), "linked change").unwrap();
    let linked = store
        .git(
            tree.id,
            &Checkpoint {
                label: "linked".into(),
            },
        )
        .unwrap();
    std::fs::write(Path::new(&tree.path).join("tracked.txt"), "later").unwrap();
    let preview = store.git(tree.id, &Inspect).unwrap();
    store
        .git(
            tree.id,
            &Restore {
                checkpoint: linked.checkpoints[0].id.clone(),
                expected: preview.version,
            },
        )
        .unwrap();
    assert_eq!(std::fs::read_to_string(Path::new(&tree.path).join("tracked.txt")).unwrap(), "linked change");
    assert_eq!(std::fs::read_to_string(fixture.repo.join("tracked.txt")).unwrap(), "original\n");
    let unborn = fixture._root.path().join("unborn");
    std::fs::create_dir(&unborn).unwrap();
    git(&unborn, &["init", "-b", "main"]);
    let settings = Settings {
        cwd: unborn.clone(),
        ..fixture.settings.clone()
    };
    let store = Projects::open(settings.clone(), settings.application().unwrap()).unwrap();
    let id = store.default.unwrap();
    std::fs::write(unborn.join("new.txt"), "first").unwrap();
    let saved = store
        .git(
            id,
            &Checkpoint {
                label: "unborn".into(),
            },
        )
        .unwrap();
    assert!(saved.version.head.is_none());
    let staged = store
        .git(
            id,
            &Stage {
                path: "new.txt".into(),
                staged: true,
                expected: saved.version,
            },
        )
        .unwrap();
    assert_eq!(staged.files[0].index_status, "A");
    let unstage = store
        .git(
            id,
            &Stage {
                path: "new.txt".into(),
                staged: false,
                expected: staged.version,
            },
        )
        .unwrap();
    assert_eq!(unstage.files[0].index_status, "?");
    std::fs::write(unborn.join("new.txt"), "second").unwrap();
    let now = store.git(id, &Inspect).unwrap();
    store
        .git(
            id,
            &Restore {
                checkpoint: saved.checkpoints[0].id.clone(),
                expected: now.version,
            },
        )
        .unwrap();
    assert_eq!(std::fs::read_to_string(unborn.join("new.txt")).unwrap(), "first");
}
#[test]
fn checkpoints_reject_hidden_changes_merge_state_and_credentials_inside_the_repo() {
    use gpui_agent_domain::GitCommand::*;
    let fixture = Fixture::new();
    let store = fixture.open();
    let id = store.default.unwrap();
    git(&fixture.repo, &["update-index", "--assume-unchanged", "tracked.txt"]);
    assert!(store.git(id, &Inspect).is_err());
    git(&fixture.repo, &["update-index", "--no-assume-unchanged", "tracked.txt"]);
    git(&fixture.repo, &["config", "core.sparseCheckout", "true"]);
    assert!(store.git(id, &Inspect).is_err());
    git(&fixture.repo, &["config", "core.sparseCheckout", "false"]);
    git(&fixture.repo, &["update-index", "--split-index"]);
    assert!(store.git(id, &Inspect).is_err());
    git(&fixture.repo, &["update-index", "--no-split-index"]);
    git(&fixture.repo, &["config", "core.splitIndex", "true"]);
    assert!(store.git(id, &Inspect).is_err());
    git(&fixture.repo, &["config", "core.splitIndex", "false"]);
    std::fs::write(fixture.repo.join(".git/MERGE_HEAD"), "in progress").unwrap();
    assert!(store.git(id, &Inspect).is_err());
    std::fs::remove_file(fixture.repo.join(".git/MERGE_HEAD")).unwrap();
    let settings = Settings {
        home: fixture.repo.join("private-home"),
        ..fixture.settings.clone()
    };
    let store = Projects::open(settings.clone(), settings.application().unwrap()).unwrap();
    assert!(
        store
            .git(
                store.default.unwrap(),
                &Checkpoint {
                    label: "must not capture credentials".into()
                }
            )
            .is_err()
    );
}

#[test]
fn poisoned_project_state_rejects_mutations_and_cache_keeps_applications_visible() {
    let fixture = Fixture::new();
    let store = fixture.open();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = store.state.lock().unwrap();
            panic!("poison project database");
        }))
        .is_err()
    );
    assert_eq!(store.catalog().unwrap_err().0, ErrorKind::Failure);
    assert!(store.create_worktree(1, "must-not-create", "HEAD").is_err());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = store.applications.lock().unwrap();
            panic!("poison workspace application cache");
        }))
        .is_err()
    );
    assert!(store.active_application(store.default).is_some());
    store.cancel_all();
}

#[test]
fn given_a_saved_commit_proposal_when_files_change_or_commit_fails_then_git_and_staging_are_preserved() {
    use gpui_agent_domain::GitCommand::*;
    let fixture = Fixture::new();
    let store = fixture.open();
    let id = store.default.unwrap();
    git(&fixture.repo, &["config", "user.name", "Commit Test"]);
    git(&fixture.repo, &["config", "user.email", "test@example.invalid"]);
    std::fs::write(fixture.repo.join("tracked.txt"), "reviewed\n").unwrap();
    std::fs::write(fixture.repo.join("unrelated.txt"), "staged\n").unwrap();
    git(&fixture.repo, &["add", "unrelated.txt"]);
    let proposed = store
        .git(
            id,
            &ProposeCommit {
                message: "proposal".into(),
                paths: vec!["tracked.txt".into()],
            },
        )
        .unwrap();
    let proposal = proposed.proposal.clone().unwrap();
    drop(store);
    let store = fixture.open();
    assert_eq!(store.git(id, &Inspect).unwrap().proposal, Some(proposal));
    let before = std::fs::read(fixture.repo.join(".git/index")).unwrap();
    let approve = CommitSelected {
        message: "reviewed".into(),
        paths: vec!["tracked.txt".into()],
        expected: proposed.version.clone(),
    };
    // A new untracked file invalidates the complete reviewed code version.
    std::fs::write(fixture.repo.join("late.txt"), "new content").unwrap();
    assert!(store.git(id, &approve).is_err());
    std::fs::remove_file(fixture.repo.join("late.txt")).unwrap();
    // Foreign index locks and invalid selections cannot overwrite the index.
    std::fs::write(fixture.repo.join(".git/index.lock"), "other owner").unwrap();
    assert!(store.git(id, &approve).is_err());
    assert_eq!(std::fs::read_to_string(fixture.repo.join(".git/index.lock")).unwrap(), "other owner");
    std::fs::remove_file(fixture.repo.join(".git/index.lock")).unwrap();
    for paths in [
        vec![],
        vec!["../outside".into()],
        vec![".".into()],
        vec!["unchanged.txt".into()],
        vec![":(glob)*".into()],
    ] {
        assert!(
            store
                .git(
                    id,
                    &CommitSelected {
                        message: "reviewed".into(),
                        paths,
                        expected: proposed.version.clone()
                    }
                )
                .is_err()
        );
    }
    // A failing commit must retain the pending proposal, HEAD and staged tree.
    git(&fixture.repo, &["config", "user.name", ""]);
    assert!(store.git(id, &approve).is_err());
    let failed = store.git(id, &Inspect).unwrap();
    assert_eq!(failed.version, proposed.version);
    assert!(failed.proposal.is_some());
    assert_eq!(std::fs::read(fixture.repo.join(".git/index")).unwrap(), before);
    assert!(!fixture.repo.join(".git/index.lock").exists());
}

#[tokio::test]
async fn given_an_active_workspace_lease_when_proposing_and_inspecting_then_neither_waits_for_the_worker_to_finish() {
    let fixture = Fixture::new();
    let service = fixture.open().service();
    let id = service.default_workspace().unwrap();
    let _active_worker = service.scope(Some(id)).await.unwrap();
    std::fs::write(fixture.repo.join("tracked.txt"), "agent edit\n").unwrap();
    let proposal = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        service.propose_commit_for_session(Some(id), None, "pending".into(), vec!["tracked.txt".into()]),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(proposal.proposal.is_some());
    let overview = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        service.git(id, gpui_agent_domain::GitCommand::Inspect, || {
            panic!("read-only inspection must not enforce idle jobs")
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(overview.version, proposal.version);
    // An attempted approval must time out instead of indefinitely blocking
    // the running worker's next proposal/read lease.
    let approval = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        service.git(
            id,
            gpui_agent_domain::GitCommand::CommitSelected {
                message: "approved".into(),
                paths: vec!["tracked.txt".into()],
                expected: proposal.version,
            },
            || Ok(()),
        ),
    )
    .await
    .expect("approval must not wait for the active worker");
    assert!(matches!(approval, Err(ApplicationError(ErrorKind::Busy, _))));
}

#[test]
fn given_a_reviewed_branch_when_another_branch_has_the_same_head_then_approval_is_stale() {
    use gpui_agent_domain::GitCommand::*;
    let fixture = Fixture::new();
    let store = fixture.open();
    let id = store.default.unwrap();
    std::fs::write(fixture.repo.join("tracked.txt"), "reviewed\n").unwrap();
    let review = store.git(id, &Inspect).unwrap();
    git(&fixture.repo, &["switch", "-c", "other-branch"]);
    let other = store.git(id, &Inspect).unwrap();
    assert_ne!(review.version, other.version, "branch identity belongs to the reviewed version");
    assert!(
        store
            .git(
                id,
                &CommitSelected {
                    message: "reviewed".into(),
                    paths: vec!["tracked.txt".into()],
                    expected: review.version
                }
            )
            .is_err()
    );
}

#[test]
fn given_a_staged_rename_and_recreated_source_when_only_the_destination_is_approved_then_recreated_content_stays_untracked() {
    use gpui_agent_domain::GitCommand::*;
    let fixture = Fixture::new();
    let store = fixture.open();
    let id = store.default.unwrap();
    git(&fixture.repo, &["config", "user.name", "Commit Test"]);
    git(&fixture.repo, &["config", "user.email", "test@example.invalid"]);
    git(&fixture.repo, &["mv", "tracked.txt", "renamed.txt"]);
    std::fs::write(fixture.repo.join("tracked.txt"), "unrelated recreated source\n").unwrap();
    let review = store.git(id, &Inspect).unwrap();
    let approved = store
        .git(
            id,
            &CommitSelected {
                message: "review rename".into(),
                paths: vec!["renamed.txt".into()],
                expected: review.version,
            },
        )
        .unwrap();
    assert_eq!(git_text(&fixture.repo, &["show", "HEAD:renamed.txt"]), "original\n");
    assert!(
        !git_text(&fixture.repo, &["ls-tree", "--name-only", "HEAD"])
            .lines()
            .any(|path| path == "tracked.txt")
    );
    assert_eq!(
        std::fs::read_to_string(fixture.repo.join("tracked.txt")).unwrap(),
        "unrelated recreated source\n"
    );
    let source = approved.files.iter().find(|file| file.path == "tracked.txt").unwrap();
    assert_eq!(source.index_status, "?");
    assert_eq!(source.worktree_status, "?");
}

#[test]
fn given_a_prepared_commit_when_head_moves_before_publication_then_the_new_head_and_index_are_preserved() {
    use gpui_agent_domain::GitCommand::*;
    let fixture = Fixture::new();
    let store = fixture.open();
    let id = store.default.unwrap();
    git(&fixture.repo, &["config", "user.name", "Commit Test"]);
    git(&fixture.repo, &["config", "user.email", "test@example.invalid"]);
    std::fs::write(fixture.repo.join("tracked.txt"), "reviewed\n").unwrap();
    let review = store
        .git(
            id,
            &ProposeCommit {
                message: "reviewed".into(),
                paths: vec!["tracked.txt".into()],
            },
        )
        .unwrap();
    let index = fixture._root.path().join("prepared-index");
    crate::git::run_env(
        &fixture.repo,
        &store.hooks,
        &["read-tree", &review.version.worktree],
        &[("GIT_INDEX_FILE", index.as_os_str())],
    )
    .unwrap()
    .require()
    .unwrap();
    let before_index = std::fs::read(fixture.repo.join(".git/index")).unwrap();
    let external = git_text(
        &fixture.repo,
        &[
            "commit-tree",
            &review.version.index,
            "-p",
            review.version.head.as_deref().unwrap(),
            "-m",
            "external commit",
        ],
    );
    git(&fixture.repo, &["update-ref", "HEAD", external.trim()]);
    // The external ref update is the event between preparation and publication.
    let result = super::checkpoints::publish_commit(&fixture.repo, &store.hooks, "reviewed", &index, &review.version);
    assert!(result.is_err(), "changed HEAD must reject publication");
    assert_eq!(git_text(&fixture.repo, &["rev-parse", "HEAD"]).trim(), external.trim());
    assert_eq!(std::fs::read(fixture.repo.join(".git/index")).unwrap(), before_index);
    assert_eq!(store.git(id, &Inspect).unwrap().proposal, review.proposal);
}

#[test]
fn given_selected_deletions_new_files_and_an_unborn_branch_when_approved_then_only_the_selected_paths_are_committed() {
    use gpui_agent_domain::GitCommand::*;
    for staged in [false, true] {
        let fixture = Fixture::new();
        let store = fixture.open();
        let id = store.default.unwrap();
        git(&fixture.repo, &["config", "user.name", "Commit Test"]);
        git(&fixture.repo, &["config", "user.email", "test@example.invalid"]);
        std::fs::remove_file(fixture.repo.join("tracked.txt")).unwrap();
        if staged {
            git(&fixture.repo, &["add", "-u"]);
        }
        std::fs::write(fixture.repo.join("new[1].txt"), "selected new\n").unwrap();
        std::fs::write(fixture.repo.join("other.txt"), "unselected\n").unwrap();
        let review = store.git(id, &Inspect).unwrap();
        store
            .git(
                id,
                &CommitSelected {
                    message: "selected deletion and new file".into(),
                    paths: vec!["tracked.txt".into(), "new[1].txt".into()],
                    expected: review.version,
                },
            )
            .unwrap();
        assert_eq!(git_text(&fixture.repo, &["show", "HEAD:new[1].txt"]), "selected new\n");
        assert_eq!(git_text(&fixture.repo, &["ls-tree", "--name-only", "HEAD"]), ".gitignore\nnew[1].txt\n");
        assert_eq!(std::fs::read_to_string(fixture.repo.join("other.txt")).unwrap(), "unselected\n");
    }
    let fixture = Fixture::new();
    let unborn = fixture._root.path().join("unborn approval");
    std::fs::create_dir(&unborn).unwrap();
    git(&unborn, &["init", "-b", "main"]);
    git(&unborn, &["config", "user.name", "Commit Test"]);
    git(&unborn, &["config", "user.email", "test@example.invalid"]);
    let settings = Settings {
        cwd: unborn.clone(),
        ..fixture.settings.clone()
    };
    let store = Projects::open(settings.clone(), settings.application().unwrap()).unwrap();
    let id = store.default.unwrap();
    std::fs::write(unborn.join("first.txt"), "first\n").unwrap();
    std::fs::write(unborn.join("other.txt"), "other\n").unwrap();
    let review = store.git(id, &Inspect).unwrap();
    assert!(review.version.head.is_none());
    let committed = store
        .git(
            id,
            &CommitSelected {
                message: "first commit".into(),
                paths: vec!["first.txt".into()],
                expected: review.version,
            },
        )
        .unwrap();
    assert!(committed.commit.is_some());
    assert_eq!(git_text(&unborn, &["ls-tree", "--name-only", "HEAD"]), "first.txt\n");
    assert_eq!(git_text(&unborn, &["rev-list", "--count", "HEAD"]).trim(), "1");
}

#[test]
fn given_a_prepared_commit_when_the_branch_changes_before_publication_then_neither_branch_is_changed() {
    use gpui_agent_domain::GitCommand::*;
    let fixture = Fixture::new();
    let store = fixture.open();
    let id = store.default.unwrap();
    git(&fixture.repo, &["config", "user.name", "Commit Test"]);
    git(&fixture.repo, &["config", "user.email", "test@example.invalid"]);
    std::fs::write(fixture.repo.join("tracked.txt"), "reviewed\n").unwrap();
    let review = store.git(id, &Inspect).unwrap();
    let index = fixture._root.path().join("prepared-index");
    crate::git::run_env(
        &fixture.repo,
        &store.hooks,
        &["read-tree", &review.version.worktree],
        &[("GIT_INDEX_FILE", index.as_os_str())],
    )
    .unwrap()
    .require()
    .unwrap();
    let before_index = std::fs::read(fixture.repo.join(".git/index")).unwrap();
    git(&fixture.repo, &["switch", "-c", "other-branch"]);
    assert!(super::checkpoints::publish_commit(&fixture.repo, &store.hooks, "reviewed", &index, &review.version).is_err());
    for branch in ["main", "other-branch"] {
        assert_eq!(git_text(&fixture.repo, &["rev-parse", branch]).trim(), review.version.head.as_deref().unwrap());
    }
    assert_eq!(std::fs::read(fixture.repo.join(".git/index")).unwrap(), before_index);
}

#[tokio::test]
async fn given_a_transient_workspace_reader_when_approval_arrives_then_it_waits_for_the_reader_to_release() {
    let fixture = Fixture::new();
    git(&fixture.repo, &["config", "user.name", "Commit Test"]);
    git(&fixture.repo, &["config", "user.email", "test@example.invalid"]);
    let service = fixture.open().service();
    let id = service.default_workspace().unwrap();
    std::fs::write(fixture.repo.join("tracked.txt"), "reviewed\n").unwrap();
    let review = service.git(id, gpui_agent_domain::GitCommand::Inspect, || Ok(())).await.unwrap();
    let transient = service.scope(Some(id)).await.unwrap();
    let (started, ready) = tokio::sync::oneshot::channel();
    let approval = tokio::spawn(async move {
        started.send(()).unwrap();
        service
            .git(
                id,
                gpui_agent_domain::GitCommand::CommitSelected {
                    message: "reviewed".into(),
                    paths: vec!["tracked.txt".into()],
                    expected: review.version,
                },
                || Ok(()),
            )
            .await
    });
    ready.await.unwrap();
    drop(transient);
    let result = tokio::time::timeout(std::time::Duration::from_secs(20), approval).await.unwrap().unwrap();
    assert!(result.is_ok(), "a completed reader must not reject approval: {result:?}");
}
