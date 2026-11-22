use gpui_agent_contracts::{McpOAuthEffect,
                           McpOAuthEvent,
                           McpOAuthMachine,
                           Transition,
                           mcp_oauth_callback,
                           mcp_oauth_refresh_due,
                           transition_mcp_oauth};

pub trait OAuthSessionActor {
    fn step_oauth(self, event: McpOAuthEvent) -> Transition<McpOAuthMachine, McpOAuthEffect>;
}

impl OAuthSessionActor for McpOAuthMachine {
    fn step_oauth(self, event: McpOAuthEvent) -> Transition<McpOAuthMachine, McpOAuthEffect> { transition_mcp_oauth(self, event) }
}

pub struct OAuthSessionContext<A: OAuthSessionActor> {
    actor: A,
}

impl<A: OAuthSessionActor> OAuthSessionContext<A> {
    pub fn new(actor: A) -> Self {
        Self {
            actor,
        }
    }

    pub fn start_login(self, authorization_url: String) -> Transition<McpOAuthMachine, McpOAuthEffect> {
        self.actor.step_oauth(McpOAuthEvent::StartLogin {
            authorization_url,
        })
    }

    pub fn receive_callback(self, request: &str, redirect: &str, state: &str, issuer: &str, issuer_required: bool) -> Result<Option<String>, String> {
        mcp_oauth_callback(request, redirect, state, issuer, issuer_required)
    }

    pub fn complete_authorization(self, expires_at: Option<u64>) -> Transition<McpOAuthMachine, McpOAuthEffect> {
        self.actor.step_oauth(McpOAuthEvent::CallbackAuthorized {
            expires_at,
        })
    }

    pub fn deny_authorization(self) -> Transition<McpOAuthMachine, McpOAuthEffect> { self.actor.step_oauth(McpOAuthEvent::CallbackDenied) }

    pub fn evaluate_refresh(self, expires_at: Option<u64>, now: u64, has_refresh: bool) -> Result<bool, Transition<McpOAuthMachine, McpOAuthEffect>> {
        match mcp_oauth_refresh_due(expires_at, now, has_refresh) {
            | Ok(due) => Ok(due),
            | Err(message) => Err(self.actor.step_oauth(McpOAuthEvent::TokenExpired {
                message,
            })),
        }
    }

    pub fn cancel(self) -> Transition<McpOAuthMachine, McpOAuthEffect> { self.actor.step_oauth(McpOAuthEvent::Cancel) }

    pub fn logout(self) -> Transition<McpOAuthMachine, McpOAuthEffect> { self.actor.step_oauth(McpOAuthEvent::Logout) }

    pub fn fail(self, message: String) -> Transition<McpOAuthMachine, McpOAuthEffect> {
        self.actor.step_oauth(McpOAuthEvent::Fail {
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_agent_contracts::McpOAuthState;

    #[test]
    fn oauth_session_context_transitions_through_login_and_logout() {
        let login = OAuthSessionContext::new(McpOAuthMachine::SignedOut).start_login("https://auth.example/authorize".into());
        assert_eq!(login.state.to_status().state, McpOAuthState::Pending);
        assert_eq!(
            login.effects,
            vec![McpOAuthEffect::OpenAuthorizationUrl("https://auth.example/authorize".into())]
        );

        let connected = OAuthSessionContext::new(login.state).complete_authorization(Some(500));
        assert_eq!(connected.state.to_status().state, McpOAuthState::Connected);
        assert_eq!(connected.state.to_status().expires_at, Some(500));

        let expired = OAuthSessionContext::new(connected.state).evaluate_refresh(Some(500), 500, false).unwrap_err();
        assert_eq!(expired.state.to_status().state, McpOAuthState::Expired);

        let logged_out = OAuthSessionContext::new(expired.state).logout();
        assert_eq!(logged_out.state.to_status().state, McpOAuthState::SignedOut);
        assert_eq!(logged_out.effects, vec![McpOAuthEffect::ClearStoredTokens]);
    }
}
