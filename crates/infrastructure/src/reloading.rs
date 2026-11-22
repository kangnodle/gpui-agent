use crate::Settings;
use gpui_agent_application::ports::{AgentRuntime,
                                    RuntimeFuture};
use gpui_agent_domain::*;
pub(crate) struct Reloading {
    pub settings: Settings,
    pub provider: Provider,
}
impl AgentRuntime for Reloading {
    fn supports_guidance_loading(&self) -> bool { self.settings.mcp_sessions.is_some() }

    fn supports_steering(&self) -> bool { self.provider == Provider::Codex }

    fn prompt_with_input<'a>(
        &'a self,
        prompt: &'a Prompt,
        handler: &'a mut Handler<'_>,
        input: &'a mut dyn gpui_agent_application::ports::TurnInput,
    ) -> RuntimeFuture<'a, Completion> {
        Box::pin(async move { self.settings.agent(self.provider)?.prompt_with_input(prompt, handler, Some(input)).await })
    }

    fn issue(&self) -> Option<String> { self.settings.agent(self.provider).err().map(|e| e.to_string()) }

    fn login<'a>(&'a self, device_code: bool, handler: &'a mut Handler<'_>) -> RuntimeFuture<'a, AuthStatus> {
        Box::pin(async move { self.settings.agent(self.provider)?.login(device_code, handler).await })
    }

    fn status(&self) -> RuntimeFuture<'_, AuthStatus> { Box::pin(async move { self.settings.agent(self.provider)?.status().await }) }

    fn logout(&self) -> RuntimeFuture<'_, ()> { Box::pin(async move { self.settings.agent(self.provider)?.logout().await }) }

    fn models(&self) -> RuntimeFuture<'_, Vec<Model>> { Box::pin(async move { self.settings.agent(self.provider)?.models().await }) }

    fn prompt<'a>(&'a self, prompt: &'a Prompt, handler: &'a mut Handler<'_>) -> RuntimeFuture<'a, Completion> {
        Box::pin(async move { self.settings.agent(self.provider)?.prompt(prompt, handler).await })
    }
}
