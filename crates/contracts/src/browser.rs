use crate::{ControlCommand,
            ControlFrame};
use serde::{Deserialize,
            Serialize};

pub const BROWSER_TEXT_LIMIT: usize = 16_000;
pub const BROWSER_TARGET_LIMIT: usize = 100;
pub const BROWSER_HTML_LIMIT: usize = 512 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserCommand {
    Open {
        url: String,
        #[serde(default)]
        mobile: bool,
    },
    RenderHtml {
        html: String,
        #[serde(default)]
        mobile: bool,
    },
    Snapshot {},
    Act {
        action: BrowserAction,
    },
    Wait {
        text: String,
        #[serde(default = "wait_timeout")]
        timeout_ms: u64,
    },
    Capture {},
    Diagnostics {},
    Close {},
}
fn wait_timeout() -> u64 { 1000 }

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserAction {
    Click { revision: String, target: usize },
    Fill { revision: String, target: usize, text: String },
    Key { key: String },
    Scroll { delta: i32 },
}
impl BrowserCommand {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            | Self::RenderHtml {
                html, ..
            } if html.trim().is_empty() || html.len() > BROWSER_HTML_LIMIT || html.contains('\0') =>
                Err("HTML must contain 1–524288 UTF-8 bytes without NUL characters"),
            | Self::Open {
                url,
                mobile,
            } => ControlCommand::Open {
                url: url.clone(),
                mobile: *mobile,
            }
            .validate(),
            | Self::Act {
                action,
            } => {
                match action {
                    | BrowserAction::Click {
                        revision,
                        target,
                    }
                    | BrowserAction::Fill {
                        revision,
                        target,
                        ..
                    } if revision.is_empty() || revision.len() > 80 || *target >= BROWSER_TARGET_LIMIT =>
                        return Err("Use a target and revision from the latest browser snapshot"),
                    | _ => {},
                }
                match action {
                    | BrowserAction::Fill {
                        text, ..
                    } => ControlCommand::Text {
                        text: text.clone(),
                    }
                    .validate(),
                    | BrowserAction::Key {
                        key,
                    } => ControlCommand::Key {
                        key: key.clone(),
                    }
                    .validate(),
                    | BrowserAction::Scroll {
                        delta,
                    } => ControlCommand::Scroll {
                        delta: *delta,
                    }
                    .validate(),
                    | _ => Ok(()),
                }
            },
            | Self::Wait {
                text,
                timeout_ms,
            } if text.trim().is_empty() || text.len() > 4096 || *timeout_ms > 10_000 => Err("Wait requires 1–4096 bytes of text and timeout_ms 0–10000"),
            | _ => Ok(()),
        }
    }
}

pub fn html_preview_document(html: &str) -> String {
    let escaped = html.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;");
    // The parent CSP is inherited by srcdoc and cannot be relaxed by supplied
    // markup.
    format!(
        r#"<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; frame-src about:; connect-src 'none'; base-uri 'none'; form-action 'none'"><style>html,body{{margin:0;width:100%;height:100%;overflow:hidden}}iframe{{display:block;width:100%;height:100%;border:0}}</style><iframe name="gpui-html-preview" title="HTML result" sandbox="allow-scripts" referrerpolicy="no-referrer" allow="camera 'none'; microphone 'none'; geolocation 'none'; clipboard-read 'none'; clipboard-write 'none'" srcdoc="{escaped}"></iframe>"#
    )
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BrowserTarget {
    pub id: usize,
    pub tag: String,
    pub kind: String,
    pub label: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct BrowserSnapshot {
    pub revision: String,
    pub url: String,
    pub title: String,
    pub text: String,
    pub text_truncated: bool,
    pub targets: Vec<BrowserTarget>,
    pub targets_truncated: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserDiagnostic {
    pub kind: String,
    pub level: String,
    pub text: String,
    pub url: String,
    pub line: Option<u64>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BrowserDiagnostics {
    pub entries: Vec<BrowserDiagnostic>,
    pub truncated: bool,
}
#[derive(Debug)]
pub enum BrowserReply {
    Snapshot(BrowserSnapshot),
    Wait { matched: bool, snapshot: BrowserSnapshot },
    Capture(ControlFrame),
    Diagnostics(BrowserDiagnostics),
    Closed,
    Rejected(&'static str),
}
