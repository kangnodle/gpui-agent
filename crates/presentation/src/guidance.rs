use gpui_agent_contracts::{GuidanceFile,
                           GuidanceFileCatalog,
                           GuidanceSnapshot,
                           GuidanceWriteRequest,
                           SkillMetadata,
                           SkillReference};
use std::collections::{BTreeMap,
                       BTreeSet};

#[derive(Default)]
pub struct GuidanceEditor {
    pub scope: String,
    pub catalog: GuidanceFileCatalog,
    pub file: Option<GuidanceFile>,
    pub text: String,
    pub user: bool,
    pub name: String,
    pub search: String,
    pub creating: bool,
    pub description: String,
    pub message: String,
    pub proposals: Vec<gpui_agent_contracts::SkillProposal>,
    pub mcp_proposals: Vec<gpui_agent_contracts::McpProposal>,
}

/// Every file an approval would write, manifest first, as (path, text).
pub fn skill_proposal_files(proposal: &gpui_agent_contracts::SkillProposal) -> Vec<(String, String)> {
    let root = proposal.skill_id.trim_start_matches("repo:").trim_end_matches("/SKILL.md");
    std::iter::once((format!("{root}/SKILL.md"), proposal.content.clone()))
        .chain(proposal.resources.iter().map(|(name, text)| (format!("{root}/{name}"), text.clone())))
        .collect()
}

/// What an MCP proposal would run or connect to, as (label, value) facts.
pub fn mcp_proposal_facts(proposal: &gpui_agent_contracts::McpProposal) -> Vec<(&'static str, String)> {
    use gpui_agent_contracts::McpServer;
    let mut facts = vec![("이름", proposal.server.name().to_owned())];
    match &proposal.server {
        | McpServer::Stdio {
            command,
            args,
            ..
        } => {
            facts.push(("실행 파일", command.clone()));
            if !args.is_empty() {
                facts.push(("인자", args.join(" ")));
            }
        },
        | McpServer::Http {
            url,
            bearer_token_env_var,
            env_http_headers,
            ..
        } => {
            facts.push(("주소", url.clone()));
            if let Some(variable) = bearer_token_env_var {
                facts.push(("Bearer 환경 변수", variable.clone()));
            }
            facts.extend(
                env_http_headers
                    .iter()
                    .map(|(header, variable)| ("헤더 환경 변수", format!("{header}={variable}"))),
            );
        },
    }
    facts.extend(
        proposal
            .environment
            .iter()
            .map(|check| (if check.ready { "환경 변수 준비됨" } else { "환경 변수 미설정" }, check.variable.clone())),
    );
    facts.push(("대화의 마지막 요청", proposal.requested_during.clone().unwrap_or_else(|| "-".into())));
    facts.push(("설치 위치", proposal.plugin_id.clone()));
    facts.push(("이유", proposal.rationale.clone()));
    facts
}

impl GuidanceEditor {
    pub fn begin(&mut self, scope: String) -> bool {
        if self.scope != scope {
            if self.dirty() {
                return false;
            }
            *self = Self {
                scope,
                ..Self::default()
            };
        }
        true
    }

    pub fn dirty(&self) -> bool { self.file.as_ref().is_some_and(|file| self.text != file.text) }

    pub fn select_scope(&mut self, user: bool) -> bool {
        if self.dirty() {
            return false;
        }
        if self.user != user {
            self.user = user;
            self.file = None;
            self.text.clear();
            self.search.clear();
            self.creating = false;
            self.name.clear();
            self.description.clear();
            self.message.clear();
        }
        true
    }

    pub fn skill_count(&self, snapshot: Option<&GuidanceSnapshot>, user: bool) -> usize {
        snapshot
            .into_iter()
            .flat_map(|snapshot| snapshot.skills.iter().map(|skill| skill.id.as_str()))
            .chain(self.catalog.files.iter().map(String::as_str))
            .filter(|id| id.ends_with("/SKILL.md") && skill_in_scope(id, user))
            .collect::<BTreeSet<_>>()
            .len()
    }

    pub fn load(&mut self, file: GuidanceFile) {
        self.text = file.text.clone();
        self.file = Some(file);
        self.message.clear();
    }

    pub fn request(&self) -> Option<GuidanceWriteRequest> {
        self.file.as_ref().map(|file| GuidanceWriteRequest {
            id: file.id.clone(),
            text: self.text.clone(),
            expected_revision: file.revision.clone(),
        })
    }

    pub fn new_id(&self, skill: bool) -> String {
        let (scope, prefix) = if self.user {
            ("user:", "")
        } else {
            ("repo:", self.catalog.project_prefix.as_str())
        };
        if skill {
            format!("{scope}{prefix}{}skills/{}/SKILL.md", if self.user { "" } else { ".agents/" }, self.name.trim())
        } else {
            format!("{scope}{prefix}AGENTS.md")
        }
    }
}

pub fn skill_in_scope(id: &str, user: bool) -> bool { id.strip_prefix("plugin:").unwrap_or(id).starts_with(if user { "user:" } else { "repo:" }) }

/// Whether one scope's AGENTS.md reaches the next turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstructionStatus {
    Checking,
    Applied,
    Truncated,
    Problem,
    Missing,
}

impl InstructionStatus {
    pub fn label(self) -> &'static str {
        match self {
            | Self::Checking => "확인 중",
            | Self::Applied => "적용 중",
            | Self::Truncated => "일부만 적용",
            | Self::Problem => "확인 필요",
            | Self::Missing => "파일 없음",
        }
    }
}

pub fn instruction_status(snapshot: Option<&GuidanceSnapshot>, user: bool) -> InstructionStatus {
    let Some(snapshot) = snapshot else {
        return InstructionStatus::Checking;
    };
    let documents: Vec<_> = snapshot.instructions.iter().filter(|doc| skill_in_scope(&doc.source, user)).collect();
    if documents.iter().any(|doc| doc.truncated) {
        InstructionStatus::Truncated
    } else if snapshot
        .diagnostics
        .iter()
        .any(|diagnostic| skill_in_scope(&diagnostic.source, user) && !diagnostic.source.ends_with("/SKILL.md"))
    {
        InstructionStatus::Problem
    } else if !documents.is_empty() {
        InstructionStatus::Applied
    } else {
        InstructionStatus::Missing
    }
}

/// A guidance file id without its internal `user:`/`repo:` scope prefix.
pub fn guidance_display_path(id: &str) -> &str { id.split_once(':').map_or(id, |(_, path)| path) }

#[derive(Default)]
pub struct GuidanceState {
    pub extensions: gpui_agent_contracts::ConversationExtensions,
    scope: String,
    pub snapshot: Option<GuidanceSnapshot>,
    pub selected: Vec<SkillReference>,
    pub open: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExtensionKind {
    Plugin,
    Skill,
    Mcp,
}

pub struct ConversationExtensionEntry {
    pub id: String,
    pub key: String,
    pub name: String,
    pub description: String,
    pub kind: ExtensionKind,
    pub selected: bool,
    pub enabled: bool,
}

pub fn guidance_scope(workspace: Option<i64>, account: Option<&str>) -> String { format!("{workspace:?}:{}", account.unwrap_or("default")) }

pub fn skill_groups(skills: &[SkillMetadata]) -> BTreeMap<&str, Vec<usize>> {
    let mut groups = BTreeMap::<_, Vec<_>>::new();
    for (index, skill) in skills.iter().enumerate() {
        let plugin = skill
            .id
            .strip_prefix("plugin:")
            .and_then(|id| id.split_once('/'))
            .map(|(id, _)| id)
            .unwrap_or("");
        groups.entry(plugin).or_default().push(index);
    }
    groups
}

impl GuidanceState {
    pub fn extension_entries(&self, plugins: &gpui_agent_contracts::PluginCatalog, mcp: &[gpui_agent_contracts::McpServer]) -> Vec<ConversationExtensionEntry> {
        let mut entries = Vec::new();
        for plugin in &plugins.plugins {
            entries.push(ConversationExtensionEntry {
                id: plugin.id.clone(),
                key: format!("plugin-{}", plugin.id),
                name: plugin.manifest.display_name.as_ref().unwrap_or(&plugin.manifest.name).clone(),
                description: plugin.manifest.description.clone(),
                kind: ExtensionKind::Plugin,
                selected: self.extensions.plugins.contains(&plugin.id),
                enabled: plugin.enabled,
            });
        }
        if let Some(snapshot) = &self.snapshot {
            for (number, skill) in snapshot.skills.iter().enumerate() {
                entries.push(ConversationExtensionEntry {
                    id: skill.id.clone(),
                    key: format!("skill-{number}"),
                    name: skill.display_name.as_ref().unwrap_or(&skill.name).clone(),
                    description: skill.description.clone(),
                    kind: ExtensionKind::Skill,
                    selected: self.selected.iter().any(|reference| reference.id == skill.id),
                    enabled: skill.enabled,
                });
            }
        }
        for server in mcp {
            entries.push(ConversationExtensionEntry {
                id: server.name().into(),
                key: format!("mcp-{}", server.name()),
                name: server.name().into(),
                description: String::new(),
                kind: ExtensionKind::Mcp,
                selected: self.extensions.mcp.iter().any(|name| name == server.name()),
                enabled: true,
            });
        }
        entries
    }

    pub fn toggle_extension(&mut self, kind: ExtensionKind, id: &str) -> Result<(), &'static str> {
        if kind == ExtensionKind::Skill {
            return self.toggle(id);
        }
        let mut next = self.extensions.clone();
        let ids = if kind == ExtensionKind::Plugin { &mut next.plugins } else { &mut next.mcp };
        if let Some(index) = ids.iter().position(|name| name == id) {
            ids.remove(index);
        } else {
            ids.push(id.into());
        }
        next.validate()?;
        self.extensions = next;
        Ok(())
    }

    pub fn begin(&mut self, scope: String) {
        if self.scope != scope {
            self.scope = scope;
            self.snapshot = None;
            self.selected.clear();
            self.extensions = Default::default();
        }
    }

    pub fn receive(&mut self, scope: &str, snapshot: GuidanceSnapshot) -> bool {
        if self.scope != scope {
            return false;
        }
        self.snapshot = Some(snapshot);
        true
    }

    pub fn toggle(&mut self, id: &str) -> Result<(), &'static str> {
        if let Some(index) = self.selected.iter().position(|skill| skill.id == id) {
            self.selected.remove(index);
            return Ok(());
        }
        if !self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.skills.iter().any(|skill| skill.id == id && skill.enabled))
        {
            return Err("Skill is unavailable; refresh the catalog");
        }
        if self.selected.len() >= 8 {
            return Err("At most 8 skills may be selected per turn");
        }
        self.selected.push(SkillReference {
            id: id.into(),
        });
        Ok(())
    }
}

#[cfg(test)]
mod proposal_tests {
    use super::*;
    use gpui_agent_contracts::{ProposalStatus,
                               SkillProposal};

    #[test]
    fn given_a_skill_proposal_when_reviewed_then_every_written_file_is_listed_manifest_first() {
        let proposal = SkillProposal {
            id: "sp1".into(),
            skill_id: "repo:.agents/skills/review/SKILL.md".into(),
            name: "review".into(),
            content: "manifest".into(),
            resources: [("references/b.md".to_owned(), "B".to_owned()), ("assets/a.txt".to_owned(), "A".to_owned())].into(),
            requested_during: None,
            status: ProposalStatus::Pending,
            message: None,
            expires_at: 1,
        };
        let paths: Vec<_> = skill_proposal_files(&proposal).into_iter().map(|(path, _)| path).collect();
        assert_eq!(
            paths,
            [
                ".agents/skills/review/SKILL.md",
                ".agents/skills/review/assets/a.txt",
                ".agents/skills/review/references/b.md"
            ]
        );
    }

    #[test]
    fn given_an_mcp_proposal_when_reviewed_then_the_exact_command_endpoint_and_environment_references_are_listed() {
        use gpui_agent_contracts::{McpEnvironmentCheck,
                                   McpProposal,
                                   McpServer};
        let proposal = |server| McpProposal {
            id: "mp1".into(),
            plugin_id: "repo:mcp-x".into(),
            server,
            rationale: "Why".into(),
            environment: vec![],
            requested_during: None,
            status: ProposalStatus::Pending,
            message: None,
            expires_at: 1,
        };
        let stdio = proposal(McpServer::Stdio {
            name: "x".into(),
            command: "/bin/x".into(),
            args: vec!["--a".into(), "b".into()],
        });
        assert_eq!(
            mcp_proposal_facts(&stdio),
            [
                ("이름", "x".to_owned()),
                ("실행 파일", "/bin/x".to_owned()),
                ("인자", "--a b".to_owned()),
                ("대화의 마지막 요청", "-".to_owned()),
                ("설치 위치", "repo:mcp-x".to_owned()),
                ("이유", "Why".to_owned())
            ]
        );
        let http = proposal(McpServer::Http {
            name: "x".into(),
            url: "https://example.com/mcp".into(),
            bearer_token_env_var: Some("TOKEN".into()),
            env_http_headers: [("X-Key".to_owned(), "KEY".to_owned())].into(),
            oauth: None,
        });
        let labels: Vec<_> = mcp_proposal_facts(&http).into_iter().map(|(label, _)| label).collect();
        assert_eq!(
            labels,
            ["이름", "주소", "Bearer 환경 변수", "헤더 환경 변수", "대화의 마지막 요청", "설치 위치", "이유"]
        );
        let mut checked = http;
        checked.environment = vec![
            McpEnvironmentCheck {
                variable: "TOKEN".into(),
                ready: true,
            },
            McpEnvironmentCheck {
                variable: "KEY".into(),
                ready: false,
            },
        ];
        let facts = mcp_proposal_facts(&checked);
        assert_eq!(facts[4], ("환경 변수 준비됨", "TOKEN".to_owned()));
        assert_eq!(facts[5], ("환경 변수 미설정", "KEY".to_owned()));
        assert_eq!(facts[6], ("대화의 마지막 요청", "-".to_owned()));
    }
}
