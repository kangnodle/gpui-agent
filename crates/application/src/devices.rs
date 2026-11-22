use crate::{ApplicationResult,
            ports::HostControl};
use gpui_agent_contracts::*;

pub async fn control_device(backend: &dyn HostControl, scope: Option<&McpScope>, command: DeviceCommand) -> ApplicationResult<DeviceReply> {
    command.validate()?;
    let reply = backend.device(scope.cloned(), command).await?;
    if let DeviceReply::Screenshot {
        width,
        height,
        image,
        ..
    } = &reply
    {
        if *width == 0 || *height == 0 || *width > 16384 || *height > 16384 || u64::from(*width) * u64::from(*height) > 32_000_000 {
            return Err("Invalid device screenshot dimensions".into());
        }
        image.validate()?;
        if !matches!(image,Attachment::Image{mime_type,..} if mime_type=="image/png") {
            return Err("Device screenshot must be PNG".into());
        }
    }
    Ok(reply)
}

pub async fn grant_device(backend: &dyn HostControl, grant: DeviceGrantRequest) -> ApplicationResult<()> {
    grant.validate()?;
    backend.grant_device(grant).await?;
    Ok(())
}
