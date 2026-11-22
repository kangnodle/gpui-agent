use serde::{Deserialize,
            Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    Codex,
    Antigravity,
}

pub const PROVIDERS: [Provider; 3] = [Provider::Claude, Provider::Codex, Provider::Antigravity];

pub fn provider_name(provider: Provider) -> &'static str {
    match provider {
        | Provider::Claude => "claude",
        | Provider::Codex => "codex",
        | Provider::Antigravity => "antigravity",
    }
}

pub fn display_name(provider: Provider) -> &'static str {
    match provider {
        | Provider::Claude => "Claude",
        | Provider::Codex => "ChatGPT / Codex",
        | Provider::Antigravity => "Gemini / Antigravity",
    }
}

pub fn provider_index(provider: Provider) -> usize {
    match provider {
        | Provider::Claude => 0,
        | Provider::Codex => 1,
        | Provider::Antigravity => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_preserves_supported_provider_names_and_indices() {
        assert_eq!(PROVIDERS.map(provider_name), ["claude", "codex", "antigravity"]);
        assert_eq!(PROVIDERS.map(provider_index), [0, 1, 2]);
    }
}
