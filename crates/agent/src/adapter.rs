use crate::{Agent,
            AuthStatus,
            Completion,
            Handler,
            Model,
            Prompt};
use gpui_agent_ports::{AgentRuntime,
                       RuntimeFuture};

impl AgentRuntime for Agent {
    fn supports_guidance_loading(&self) -> bool { self.mcp.is_some() || self.mcp_issuer.is_some() }

    fn supports_steering(&self) -> bool { self.config.provider == crate::Provider::Codex }

    fn prompt_with_input<'a>(
        &'a self,
        prompt: &'a Prompt,
        handler: &'a mut Handler<'_>,
        input: &'a mut dyn gpui_agent_ports::TurnInput,
    ) -> RuntimeFuture<'a, Completion> {
        Box::pin(self.prompt_with_input(prompt, handler, Some(input)))
    }

    fn login<'a>(&'a self, device_code: bool, handler: &'a mut Handler<'_>) -> RuntimeFuture<'a, AuthStatus> { Box::pin(self.login(device_code, handler)) }

    fn status(&self) -> RuntimeFuture<'_, AuthStatus> { Box::pin(self.status()) }

    fn logout(&self) -> RuntimeFuture<'_, ()> { Box::pin(self.logout()) }

    fn models(&self) -> RuntimeFuture<'_, Vec<Model>> { Box::pin(self.models()) }

    fn prompt<'a>(&'a self, prompt: &'a Prompt, handler: &'a mut Handler<'_>) -> RuntimeFuture<'a, Completion> { Box::pin(self.prompt(prompt, handler)) }
}
