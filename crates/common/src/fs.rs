use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const TEMP_SUFFIX: &str = ".tmp";

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn read_to_string_if_exists(path: impl AsRef<Path>) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[derive(Debug, Clone)]
pub struct AtomicWriteOptions {
    mode: Option<u32>,
    create_parents: bool,
}

impl Default for AtomicWriteOptions {
    fn default() -> Self {
        Self {
            mode: None,
            create_parents: true,
        }
    }
}

impl AtomicWriteOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mode(mut self, mode: u32) -> Self {
        self.mode = Some(mode);
        self
    }

    pub fn create_parents(mut self, create: bool) -> Self {
        self.create_parents = create;
        self
    }
}

pub fn write_atomic(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
    write_atomic_with(path.as_ref(), &AtomicWriteOptions::default(), |w| {
        w.write_all(contents.as_ref())
    })
}

/// Pretty JSON with two-space indentation, the layout of the data files, written atomically.
pub fn write_json_pretty<T: serde::Serialize + ?Sized>(
    path: impl AsRef<Path>,
    value: &T,
) -> io::Result<()> {
    write_atomic_with(path.as_ref(), &AtomicWriteOptions::default(), |w| {
        Ok(serde_json::to_writer_pretty(w, value)?)
    })
}

/// Replaces `path` through a synced temp file; keeps the old file's permissions unless a mode is set.
pub fn write_atomic_with<E, F>(path: &Path, options: &AtomicWriteOptions, write: F) -> Result<(), E>
where
    E: From<io::Error>,
    F: FnOnce(&mut BufWriter<File>) -> Result<(), E>,
{
    if options.create_parents {
        create_parent_dirs(path)?;
    }
    let dir = parent_dir(path);
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?
        .to_string_lossy()
        .into_owned();

    let mode = options.mode.or_else(|| existing_mode(path));
    let (temp_path, file) =
        create_temp(&dir, &file_name, if mode.is_some() { 0o600 } else { 0o666 })?;
    let guard = TempGuard {
        path: Some(temp_path.clone()),
    };
    if let Some(mode) = mode {
        apply_mode(&file, mode)?;
    }

    let mut writer = BufWriter::new(file);
    write(&mut writer)?;
    let file = writer.into_inner().map_err(|e| e.into_error())?;
    file.sync_all()?;
    drop(file);

    fs::rename(&temp_path, path)?;
    guard.disarm();
    sync_dir(&dir);
    Ok(())
}

pub fn set_mode(path: impl AsRef<Path>, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

pub fn remove_stale_temp_files(path: impl AsRef<Path>) -> io::Result<usize> {
    let path = path.as_ref();
    let dir = parent_dir(path);
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Ok(0);
    };
    let prefix = format!(".{name}.");
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let entry_name = entry.file_name().to_string_lossy().into_owned();
        if entry_name.starts_with(&prefix)
            && entry_name.ends_with(TEMP_SUFFIX)
            && entry.file_type()?.is_file()
        {
            fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn create_parent_dirs(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => fs::create_dir_all(p),
        _ => Ok(()),
    }
}

fn create_temp(dir: &Path, file_name: &str, create_mode: u32) -> io::Result<(PathBuf, File)> {
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = dir.join(format!(
            ".{file_name}.{}-{n}{TEMP_SUFFIX}",
            std::process::id()
        ));
        let mut open = OpenOptions::new();
        open.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open.mode(create_mode);
        }
        #[cfg(not(unix))]
        let _ = create_mode;
        match open.open(&temp) {
            Ok(f) => return Ok((temp, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

fn existing_mode(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .ok()
            .map(|m| m.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

fn apply_mode(file: &File, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (file, mode);
        Ok(())
    }
}

fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    {
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
}

struct TempGuard {
    path: Option<PathBuf>,
}

impl TempGuard {
    fn disarm(mut self) {
        self.path = None;
    }
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if let Some(p) = self.path.take() {
            let _ = fs::remove_file(p);
        }
    }
}
