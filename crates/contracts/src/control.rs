use crate::Attachment;
use serde::{Deserialize,
            Serialize};

/// Manual, host-owned controls. These do not grant native AI tool permissions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ControlStatus {
    pub browser_available: bool,
    pub browser_running: bool,
    pub desktop_available: bool,
    pub desktop_enabled_until: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ControlFrame {
    pub width: u32,
    pub height: u32,
    pub url: Option<String>,
    pub image: Attachment,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlCommand {
    Open {
        url: String,
        #[serde(default)]
        mobile: bool,
    },
    Capture {},
    Click {
        x: u32,
        y: u32,
    },
    Text {
        text: String,
    },
    Key {
        key: String,
    },
    Scroll {
        delta: i32,
    },
    Reload {},
    Stop {},
}
impl ControlCommand {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            | Self::Open {
                url, ..
            } if url.len() > 8192 || url.chars().any(char::is_control) => Err("Invalid browser URL"),
            | Self::Text {
                text,
            } if text.len() > 4096 || text.chars().any(|c| c.is_control() && c != '\n') => Err("Text must be at most 4096 bytes without control characters"),
            | Self::Key {
                key,
            } if !matches!(
                key.as_str(),
                "Enter" | "Tab" | "Escape" | "Backspace" | "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" | "Home" | "End" | "Delete"
            ) =>
                Err("Unsupported key"),
            | Self::Scroll {
                delta,
            } if delta.abs_diff(0) > 2400 => Err("Scroll exceeds 2400"),
            | Self::Click {
                x,
                y,
            } if *x > 32768 || *y > 32768 => Err("Invalid coordinates"),
            | _ => Ok(()),
        }
    }
}
