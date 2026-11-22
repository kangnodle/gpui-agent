//! Shared guidance use cases. Host effects are supplied through GuidanceSource.
use crate::{Application,
            ApplicationError,
            ApplicationResult,
            ErrorKind,
            ports::GuidanceSource};
use gpui_agent_domain::{GuidanceFile,
                        GuidanceFileCatalog,
                        GuidanceSnapshot,
                        GuidanceWriteRequest,
                        MAX_SKILL_BYTES,
                        PreparedGuidance,
                        SkillReference,
                        SkillResource,
                        validate_guidance_file_id,
                        validate_skill_resource};
use std::sync::Arc;

#[derive(Clone)]
pub struct Guidance(Arc<dyn GuidanceSource>);

impl Guidance {
    pub fn new(source: Arc<dyn GuidanceSource>) -> Self { Self(source) }

    pub async fn import_skill(&self, request: gpui_agent_domain::SkillImportRequest) -> ApplicationResult<GuidanceFile> {
        request.validate()?;
        let source = self.0.clone();
        tokio::task::spawn_blocking(move || source.import_skill(&request))
            .await
            .map_err(anyhow::Error::from)?
            .map_err(|error| ApplicationError(ErrorKind::Invalid, error.to_string()))
    }

    pub async fn files(&self) -> ApplicationResult<GuidanceFileCatalog> {
        let source = self.0.clone();
        tokio::task::spawn_blocking(move || source.files())
            .await
            .map_err(anyhow::Error::from)?
            .map_err(ApplicationError::from)
    }

    pub async fn read_file(&self, id: &str) -> ApplicationResult<GuidanceFile> {
        validate_guidance_file_id(id)?;
        let source = self.0.clone();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || source.read_file(&id))
            .await
            .map_err(anyhow::Error::from)?
            .map_err(|error| ApplicationError(ErrorKind::Invalid, error.to_string()))
    }

    pub async fn write_file(&self, request: GuidanceWriteRequest) -> ApplicationResult<GuidanceFile> {
        request.validate()?;
        let source = self.0.clone();
        tokio::task::spawn_blocking(move || source.write_file(&request))
            .await
            .map_err(anyhow::Error::from)?
            .map_err(|error| ApplicationError(ErrorKind::Invalid, error.to_string()))
    }

    pub async fn snapshot(&self) -> ApplicationResult<GuidanceSnapshot> {
        let source = self.0.clone();
        tokio::task::spawn_blocking(move || source.snapshot())
            .await
            .map_err(anyhow::Error::from)?
            .map_err(ApplicationError::from)
    }

    pub async fn read_skill(&self, id: &str, resource: &str) -> ApplicationResult<SkillResource> {
        validate_skill_resource(resource)?;
        let source = self.0.clone();
        let id = id.to_owned();
        let resource = resource.to_owned();
        tokio::task::spawn_blocking(move || source.read_skill(&id, &resource))
            .await
            .map_err(anyhow::Error::from)?
            .map_err(|error| ApplicationError(ErrorKind::Invalid, error.to_string()))
    }

    pub async fn set_skill_enabled(&self, id: &str, enabled: bool) -> ApplicationResult<()> {
        let source = self.0.clone();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || source.set_skill_enabled(&id, enabled))
            .await
            .map_err(anyhow::Error::from)?
            .map_err(|error| ApplicationError(ErrorKind::Invalid, error.to_string()))
    }

    pub async fn prepare(&self, text: &str, references: &[SkillReference], implicit_available: bool) -> ApplicationResult<PreparedGuidance> {
        self.prepare_for_conversation(text, references, implicit_available, None).await
    }

    pub async fn prepare_for_conversation(
        &self,
        text: &str,
        references: &[SkillReference],
        implicit_available: bool,
        extensions: Option<&gpui_agent_domain::ConversationExtensions>,
    ) -> ApplicationResult<PreparedGuidance> {
        let mut snapshot = self.snapshot().await?;
        if let Some(selection) = extensions {
            selection.validate()?;
            snapshot.skills.retain(|skill| selection.allows_skill(&skill.id, references));
        }
        let choices = snapshot.select_skills(text, references)?;
        let mut selected = Vec::new();
        let mut used = 0;
        for choice in choices {
            let resource = self.read_skill(&choice.id, "SKILL.md").await?;
            if resource.skill.revision != choice.revision {
                return Err("Skill changed during preparation; refresh and retry".into());
            }
            used += resource.text.len();
            if used > MAX_SKILL_BYTES {
                return Err("Selected skills exceed the 64 KiB turn budget".into());
            }
            selected.push(resource);
        }
        Ok(PreparedGuidance {
            extensions: extensions.cloned(),
            snapshot,
            selected,
            implicit_available,
        })
    }
}

impl Application {
    pub fn with_guidance(mut self, guidance: Guidance) -> Self {
        Arc::get_mut(&mut self.0).expect("configure before cloning").guidance = Some(guidance);
        self
    }

    pub fn guidance(&self) -> ApplicationResult<Guidance> {
        self.0
            .guidance
            .clone()
            .ok_or(ApplicationError(ErrorKind::Unavailable, "Host guidance is unavailable".into()))
    }
}

impl crate::Jobs {
    pub async fn guidance_scope(&self, caller: &gpui_agent_domain::McpScope) -> ApplicationResult<crate::AccountScope> {
        let (_, scope) = self
            .conversation_scope(
                Some(caller),
                gpui_agent_contracts::ConversationTarget {
                    provider: caller.provider,
                    account: Some(caller.account_id.clone()),
                    workspace: None,
                },
            )
            .await?;
        Ok(scope)
    }
}
