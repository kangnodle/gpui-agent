use anyhow::Result;
use gpui_agent_domain::{AuthStatus,
                        Completion,
                        Handler,
                        Model,
                        Prompt};
use std::{future::Future,
          pin::Pin};
pub type RuntimeFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

#[derive(Clone)]
pub struct RuntimeInput {
    pub message_id: i64,
    pub text: String,
}
pub trait TurnInput: Send {
    /// Cancellation before the future returns must leave the input available.
    fn next(&mut self) -> RuntimeFuture<'_, Option<RuntimeInput>>;
}

/// Dropping an operation future must stop its owned runtime processes.
pub trait AgentRuntime: Send + Sync {
    fn supports_guidance_loading(&self) -> bool { false }
    fn supports_steering(&self) -> bool { false }
    fn prompt_with_input<'a>(&'a self, prompt: &'a Prompt, handler: &'a mut Handler<'_>, _input: &'a mut dyn TurnInput) -> RuntimeFuture<'a, Completion> {
        self.prompt(prompt, handler)
    }
    fn issue(&self) -> Option<String> { None }
    fn login<'a>(&'a self, device_code: bool, handler: &'a mut Handler<'_>) -> RuntimeFuture<'a, AuthStatus>;
    fn status(&self) -> RuntimeFuture<'_, AuthStatus>;
    fn logout(&self) -> RuntimeFuture<'_, ()>;
    fn models(&self) -> RuntimeFuture<'_, Vec<Model>>;
    fn prompt<'a>(&'a self, prompt: &'a Prompt, handler: &'a mut Handler<'_>) -> RuntimeFuture<'a, Completion>;
}
