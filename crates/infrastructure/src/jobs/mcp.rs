//! Common MCP configuration and host file locking.
use super::*;
use gpui_agent_contracts::McpServer;

impl JobStore {
    pub(super) fn mcp_config(&self, scope: &JobScope, servers: Option<&[McpServer]>) -> ApplicationResult<Vec<McpServer>> {
        self.db(|db| {
            account(db, scope)?;
            let path = self.home.join("mcp.json");
            if let Some(servers) = servers {
                gpui_agent_domain::validate_mcp(servers).map_err(anyhow::Error::msg)?;
                let pending: bool = db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM supported_jobs WHERE state IN ('queued','running','waiting'))",
                    [],
                    |r| r.get(0),
                )?;
                if pending {
                    return Err(reject(
                        ErrorKind::Busy,
                        "Finish or cancel all profiles' jobs before changing common MCP settings",
                    ));
                }
                let profiles = db
                    .prepare("SELECT provider,id FROM accounts ORDER BY provider,id")?
                    .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut locks = vec![];
                for (provider, account) in profiles {
                    gpui_agent_domain::validate_account_id(&account).map_err(anyhow::Error::msg)?;
                    let home = if account == "default" {
                        self.home.clone()
                    } else {
                        self.home.join("accounts").join(account)
                    }
                    .join(provider);
                    std::fs::create_dir_all(&home)?;
                    let lock = std::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .open(home.join(".operation.lock"))?;
                    lock.try_lock().map_err(|_| reject(ErrorKind::Busy, "A provider profile is in use"))?;
                    locks.push(lock);
                }
                let plugins = crate::PluginFiles::new(&self.home, Path::new(&scope.workspace_path))?;
                let _catalog_lock = plugins.lock()?;
                let mut paths = vec![scope.workspace_path.clone()];
                paths.extend(
                    self.workspaces
                        .operation_catalog()?
                        .projects
                        .into_iter()
                        .flat_map(|project| project.workspaces)
                        .filter(|workspace| !workspace.missing)
                        .map(|workspace| workspace.path),
                );
                paths.sort();
                paths.dedup();
                for path in paths {
                    let mut effective = servers.to_vec();
                    effective.extend(crate::PluginFiles::new(&self.home, Path::new(&path))?.mcp_servers()?);
                    gpui_agent_domain::validate_mcp(&effective).map_err(anyhow::Error::msg)?;
                }
                for server in servers {
                    if let McpServer::Stdio {
                        command, ..
                    } = server
                        && !Path::new(command).is_file()
                    {
                        return Err(reject(ErrorKind::Invalid, "MCP executable does not exist on the API host"));
                    }
                }
                let mut file = tempfile::NamedTempFile::new_in(&self.home)?;
                serde_json::to_writer(&mut file, servers)?;
                file.as_file().sync_all()?;
                file.persist(&path).map_err(|e| e.error)?;
            }
            gpui_agent::load_mcp_config(&self.home)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_agent_application::ports::{PluginSource,
                                        WorkspaceBackend};

    #[test]
    fn given_another_workspaces_enabled_plugin_when_common_mcp_is_saved_then_conflicts_are_rejected_without_writes() -> Result<()> {
        let root = tempfile::tempdir()?;
        let first = root.path().join("first");
        let second = root.path().join("second");
        std::fs::create_dir_all(&first)?;
        std::fs::create_dir_all(&second)?;
        let settings = Settings::discover(first.clone(), Some(root.path().join("home")))?;
        let projects = crate::Projects::open(settings.clone(), settings.application()?)?;
        projects.register(&second.to_string_lossy())?;
        let workspaces = projects.service();
        let scope = JobScope {
            provider: Provider::Claude,
            account_id: "default".into(),
            workspace_id: workspaces.default_workspace(),
            workspace_path: first.to_string_lossy().into(),
        };
        let plugin = crate::PluginFiles::new(&settings.home, &second)?;
        plugin.import(&gpui_agent_domain::PluginImportRequest {
            scope: "repo".into(),
            files: std::collections::BTreeMap::from([(
                "plugin.json".into(),
                br#"{"name":"review","description":"Review","mcp_servers":[{"name":"tools","url":"https://plugin.invalid/mcp"}]}"#.to_vec(),
            )]),
        })?;
        plugin.set_enabled("repo:review", true)?;
        let store = JobStore::open(settings.clone(), workspaces)?;
        let servers = serde_json::from_value::<Vec<McpServer>>(serde_json::json!([{"name":"tools","url":"https://common.invalid/mcp"}]))?;
        assert!(
            store.mcp_config(&scope, Some(&servers)).is_err(),
            "Common changes must validate every registered workspace"
        );
        assert!(!settings.home.join("mcp.json").exists());
        assert!(plugin.catalog()?.plugins[0].enabled);
        Ok(())
    }

    #[test]
    fn given_plugin_activation_holds_the_catalog_lock_when_common_mcp_is_saved_then_it_waits_for_the_same_lock() -> Result<()> {
        let root = tempfile::tempdir()?;
        let settings = Settings::discover(root.path().into(), Some(root.path().join("home")))?;
        let workspaces = settings.workspaces(settings.application()?)?;
        let scope = JobScope {
            provider: Provider::Claude,
            account_id: "default".into(),
            workspace_id: workspaces.default_workspace(),
            workspace_path: root.path().to_string_lossy().into(),
        };
        let store = JobStore::open(settings.clone(), workspaces)?;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(settings.home.join(".plugins.lock"))?;
        lock.lock()?;
        let (tx, rx) = std::sync::mpsc::channel();
        let (started, ready) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            started.send(()).unwrap();
            tx.send(store.mcp_config(&scope, Some(&[]))).unwrap();
        });
        ready.recv_timeout(std::time::Duration::from_secs(5))?;
        assert!(
            matches!(
                rx.recv_timeout(std::time::Duration::from_secs(2)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ),
            "Common save must not finish while the plugin catalog is locked"
        );
        assert!(!settings.home.join("mcp.json").exists());
        lock.unlock()?;
        rx.recv_timeout(std::time::Duration::from_secs(5))??;
        worker.join().unwrap();
        Ok(())
    }
}
