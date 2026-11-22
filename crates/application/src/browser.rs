use crate::{ApplicationError,
            ApplicationResult,
            ErrorKind,
            ports::HostControl};
use gpui_agent_contracts::*;

pub async fn automate_browser(backend: &dyn HostControl, scope: Option<&McpScope>, command: BrowserCommand) -> ApplicationResult<BrowserReply> {
    command.validate()?;
    let reply = backend.automate(scope.cloned(), command).await?;
    match &reply {
        | BrowserReply::Rejected(message) => return Err(ApplicationError(ErrorKind::Invalid, (*message).into())),
        | BrowserReply::Capture(frame) => {
            frame.image.validate()?;
            if !matches!(frame.image, Attachment::Image { .. }) {
                return Err("Browser capture must contain an image".into());
            }
        },
        | _ => {},
    }
    Ok(reply)
}
