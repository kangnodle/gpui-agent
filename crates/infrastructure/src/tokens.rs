use crate::{PrivateFileIdentity,
            new_token};
use anyhow::{Context,
             Result,
             ensure};
use std::{fs::{File,
               OpenOptions},
          io::{self,
               Read,
               Write},
          path::Path};

const MAX_TOKEN_FILE_BYTES: u64 = 4096;

pub fn read_private_token(path: &Path) -> Result<String> { read_token(&open_token(path)?) }

pub fn load_or_create_token(path: &Path) -> Result<String> { load_host_token(path).map(|(token, _)| token) }

pub(crate) fn load_host_token(path: &Path) -> Result<(String, PrivateFileIdentity)> {
    match open_token(path) {
        | Ok(file) => return read_host_token(file),
        | Err(error) if error.kind() == io::ErrorKind::NotFound => {},
        | Err(error) => return Err(error.into()),
    }
    let parent = path.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let pending = tempfile::Builder::new().make_in(parent, create_private_file)?;
    pending.as_file().lock()?;
    let token = new_token()?;
    match pending.persist_noclobber(path) {
        | Ok(mut file) => {
            file.write_all(token.as_bytes())?;
            file.unlock()?;
            Ok((token, PrivateFileIdentity::from_file(&file)?))
        },
        | Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => read_host_token(open_token(path)?),
        | Err(error) => Err(error.error.into()),
    }
}

fn read_host_token(file: File) -> Result<(String, PrivateFileIdentity)> {
    let token = read_token(&file)?;
    Ok((token, PrivateFileIdentity::from_file(&file)?))
}

fn open_token(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use rustix::fs::OFlags;
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY).bits() as i32);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        clack_private_fs::validate_file_path(path)?;
        options.custom_flags(winx::file::Flags::FILE_FLAG_OPEN_REPARSE_POINT.bits());
    }
    options.open(path)
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)
}

#[cfg(windows)]
use clack_private_fs::create_new_file as create_private_file;

fn read_token(file: &File) -> Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.nlink() == 1,
            "API token requires a regular file with a single link"
        );
        ensure!(
            metadata.uid() == rustix::process::geteuid().as_raw() && metadata.mode() & 0o077 == 0,
            "API token must be owned by the current user and private to that user"
        );
    }
    #[cfg(windows)]
    private_fs::verify(file).context("API token requires a regular file with a protected owner-only ACL")?;
    file.lock_shared()?;
    let mut bytes = Vec::new();
    file.take(MAX_TOKEN_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    file.unlock()?;
    ensure!(bytes.len() as u64 <= MAX_TOKEN_FILE_BYTES, "API token file is too large");
    let token = std::str::from_utf8(&bytes).context("Invalid API token file")?.trim();
    ensure!(
        token.len() >= 32 && token.bytes().all(|byte| byte.is_ascii_alphanumeric()),
        "Invalid API token file"
    );
    Ok(token.to_owned())
}
