use anyhow::{Context,
             Result,
             ensure};
use cap_fs_ext::{MetadataExt,
                 OpenOptionsMaybeDirExt};
use cap_std::{ambient_authority,
              fs::{Dir,
                   OpenOptions}};
use std::{fs::File,
          io,
          path::{Path,
                 PathBuf}};

pub(super) struct Workspace {
    path: PathBuf,
    identity: (u64, u64),
    private_paths: Vec<PathBuf>,
    private_files: Vec<PrivateFileIdentity>,
}

pub(super) struct Entry {
    pub file: File,
    pub path: PathBuf,
    pub directory: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PrivateFileIdentity {
    device: u64,
    inode: u64,
}

impl PrivateFileIdentity {
    pub(crate) fn from_file(file: &File) -> io::Result<Self> { Ok(Self::from_metadata(&cap_std::fs::Metadata::from_file(file)?)) }

    fn existing(path: &Path) -> io::Result<Option<Self>> {
        let metadata = match path.metadata() {
            | Ok(metadata) => metadata,
            | Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            | Err(error) => return Err(error),
        };
        if !metadata.is_file() {
            return Ok(None);
        }
        #[cfg(unix)]
        {
            Ok(Some(Self {
                device: metadata.dev(),
                inode: metadata.ino(),
            }))
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let file = File::options().access_mode(winx::file::AccessMode::FILE_READ_ATTRIBUTES.bits()).open(path)?;
            Self::from_file(&file).map(Some)
        }
    }

    fn from_metadata(metadata: &cap_std::fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

impl Workspace {
    pub fn new(selected_path: &Path, private_paths: &[PathBuf], identities: &[PrivateFileIdentity]) -> Result<Self> {
        let root = Dir::open_ambient_dir(selected_path, ambient_authority())?;
        let metadata = root.dir_metadata()?;
        let path = opened_path(&root.into_std_file())?;
        ensure!(comparable_path(selected_path) == comparable_path(&path), "Selected workspace changed");
        let mut protected = Vec::new();
        let mut private_files = identities.to_vec();
        for private in private_paths {
            let absolute = std::path::absolute(private)?;
            let canonical = canonical_private_path(&absolute)?;
            if let Some(identity) = PrivateFileIdentity::existing(&canonical)?
                && !private_files.contains(&identity)
            {
                private_files.push(identity);
            }
            if comparable_path(&canonical) != comparable_path(&absolute) {
                protected.push(absolute);
            }
            protected.push(canonical);
        }
        Ok(Self {
            path,
            identity: (metadata.dev(), metadata.ino()),
            private_paths: protected,
            private_files,
        })
    }

    fn root_directory(&self) -> Result<Dir> {
        let root = Dir::open_ambient_dir(&self.path, ambient_authority())?;
        let metadata = root.dir_metadata()?;
        ensure!(self.identity == (metadata.dev(), metadata.ino()), "Selected workspace changed");
        Ok(root)
    }

    pub fn open(&self, relative: &Path) -> Result<Entry> { self.open_relative(&self.root_directory()?, relative, relative) }

    pub fn open_relative(&self, parent: &Dir, name: &Path, relative: &Path) -> Result<Entry> {
        self.authorize(&self.path.join(relative))?;
        let mut options = OpenOptions::new();
        options.read(true).maybe_dir(true);
        #[cfg(unix)]
        {
            use cap_fs_ext::OpenOptionsExt;
            use rustix::fs::OFlags;
            options.custom_flags((OFlags::NONBLOCK | OFlags::NOCTTY).bits() as i32);
        }
        let file = match parent.open_with(directory_or_name(name), &options) {
            | Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                let target = self.path.join(relative).canonicalize().context("Cannot resolve workspace entry")?;
                self.authorize(&target)?;
                let target = target.strip_prefix(&self.path)?;
                self.root_directory()?.open_with(directory_or_name(target), &options)
            },
            | result => result,
        }
        .context("Cannot open workspace entry")?;
        let file = file.into_std();
        let path = opened_path(&file)?;
        self.authorize(&path)?;
        let metadata = cap_std::fs::Metadata::from_file(&file)?;
        ensure!(
            !self.private_files.contains(&PrivateFileIdentity::from_metadata(&metadata)),
            "Private workspace state is unavailable for file context"
        );
        ensure!(
            metadata.is_dir() || (metadata.is_file() && metadata.nlink() == 1),
            "Context requires a directory or a regular file without hard-link aliases"
        );
        Ok(Entry {
            file,
            path,
            directory: metadata.is_dir(),
        })
    }

    fn authorize(&self, path: &Path) -> Result<()> {
        ensure!(path.starts_with(&self.path), "Context file escapes the selected workspace");
        ensure!(
            !self.private_paths.iter().any(|private| within_private_path(path, private)) && !is_private_location(path),
            "Private workspace state is unavailable for file context"
        );
        Ok(())
    }
}

fn directory_or_name(path: &Path) -> &Path { if path.as_os_str().is_empty() { Path::new(".") } else { path } }

#[cfg(windows)]
fn comparable_path(path: &Path) -> &Path { dunce::simplified(path) }

#[cfg(unix)]
fn comparable_path(path: &Path) -> &Path { path }

#[cfg(windows)]
fn within_private_path(path: &Path, private: &Path) -> bool {
    let mut parts = comparable_path(path).components();
    comparable_path(private)
        .components()
        .all(|required| parts.next().is_some_and(|part| part.as_os_str().eq_ignore_ascii_case(required.as_os_str())))
}

#[cfg(unix)]
fn within_private_path(path: &Path, private: &Path) -> bool { path.starts_with(private) }

fn is_private_location(path: &Path) -> bool {
    let mut development = false;
    for component in path.components() {
        let name = component.as_os_str();
        if name.eq_ignore_ascii_case(".git") || name.eq_ignore_ascii_case(".gpui-agent-dev") || (development && name.eq_ignore_ascii_case("home")) {
            return true;
        }
        development = name.eq_ignore_ascii_case(".dev");
    }
    false
}

fn canonical_private_path(path: &Path) -> io::Result<PathBuf> {
    match path.canonicalize() {
        | Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let path = std::path::absolute(path)?;
            let Some((parent, name)) = path.parent().zip(path.file_name()) else {
                return Err(error);
            };
            Ok(canonical_private_path(parent)?.join(name))
        },
        | result => result,
    }
}

#[cfg(windows)]
fn opened_path(file: &File) -> io::Result<PathBuf> { winx::file::get_file_path(file) }

#[cfg(all(unix, target_vendor = "apple"))]
fn opened_path(file: &File) -> io::Result<PathBuf> {
    use std::{ffi::OsString,
              os::unix::ffi::OsStringExt};
    Ok(OsString::from_vec(rustix::fs::getpath(file)?.into_bytes()).into())
}

#[cfg(all(unix, not(target_vendor = "apple")))]
fn opened_path(file: &File) -> io::Result<PathBuf> {
    use std::os::fd::AsRawFd;
    std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
}
