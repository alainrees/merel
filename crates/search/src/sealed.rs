//! Where the index lives: decrypted in memory only, and on disk one sealed file per index file.
//!
//! Tantivy writes a file once and never changes it afterwards (only `meta.json` and
//! `.managed.json` are replaced whole), so every file can be sealed on its own as soon as it is
//! finished. Reading serves the copy in memory; nothing decrypted ever reaches the disk, and a
//! copied index directory is unreadable without the key in the keychain.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tantivy::directory::error::{DeleteError, OpenReadError, OpenWriteError};
use tantivy::directory::{AntiCallToken, FileHandle, RamDirectory, TerminatingWrite, WatchCallback, WatchHandle, WritePtr};
use tantivy::{Directory, TantivyError};

/// Seals and opens the index files; the engine backs it with the offline cache's key.
pub trait Sealer: Send + Sync {
    /// The sealed form of `plain`, bound to the file `name`.
    fn seal(&self, name: &str, plain: &[u8]) -> io::Result<Vec<u8>>;
    /// The plain form of a file sealed under `name`; `None` when it does not open.
    fn open(&self, name: &str, sealed: Vec<u8>) -> Option<Vec<u8>>;
}

const EXTENSION: &str = "sealed";

#[derive(Clone)]
struct Disk {
    dir: PathBuf,
    sealer: Arc<dyn Sealer>,
}

impl std::fmt::Debug for Disk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Disk").field("dir", &self.dir).finish_non_exhaustive()
    }
}

/// A [`RamDirectory`] whose finished files are also written sealed to a directory on disk.
#[derive(Clone, Debug)]
pub struct SealedDirectory {
    ram: RamDirectory,
    disk: Option<Disk>,
}

/// The file name on disk for an index file, never trusting the name as a path.
fn disk_name(path: &Path) -> String {
    let name = path.to_string_lossy();
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect();
    format!("{safe}.{EXTENSION}")
}

/// Lock files only matter while the program runs.
fn is_lock(path: &Path) -> bool {
    path.to_string_lossy().ends_with(".lock")
}

impl SealedDirectory {
    /// Only in memory: for tests, and when there is no key.
    pub fn in_memory() -> SealedDirectory {
        SealedDirectory { ram: RamDirectory::create(), disk: None }
    }

    /// Reads every sealed file in `dir` into memory. When one does not open (another key, a
    /// changed file) the whole directory is emptied: the index is built again from the mail,
    /// which is better than a half index.
    pub fn open(dir: &Path, sealer: Arc<dyn Sealer>) -> io::Result<SealedDirectory> {
        std::fs::create_dir_all(dir)?;
        let ram = RamDirectory::create();
        let mut broken = false;
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let Some(file) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
            let Some(name) = file.strip_suffix(&format!(".{EXTENSION}")) else { continue };
            match sealer.open(name, std::fs::read(&path)?) {
                Some(plain) => ram.atomic_write(Path::new(name), &plain)?,
                None => {
                    broken = true;
                    break;
                }
            }
        }
        if broken {
            log::warn!("{}: search index unreadable, building it again", dir.display());
            wipe(dir)?;
            return Ok(SealedDirectory { ram: RamDirectory::create(), disk: Some(Disk { dir: dir.to_path_buf(), sealer }) });
        }
        Ok(SealedDirectory { ram, disk: Some(Disk { dir: dir.to_path_buf(), sealer }) })
    }

    /// Bytes the index takes on disk (sealed), or in memory when there is no disk.
    pub fn size(&self) -> u64 {
        let Some(disk) = &self.disk else { return 0 };
        std::fs::read_dir(&disk.dir)
            .map(|entries| entries.filter_map(Result::ok).filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum())
            .unwrap_or(0)
    }

    fn persist(&self, path: &Path, plain: &[u8]) -> io::Result<()> {
        let Some(disk) = &self.disk else { return Ok(()) };
        if is_lock(path) {
            return Ok(());
        }
        let name = disk_name(path);
        let sealed = disk.sealer.seal(name.trim_end_matches(&format!(".{EXTENSION}")), plain)?;
        let target = disk.dir.join(&name);
        let tmp = disk.dir.join(format!("{name}.tmp"));
        std::fs::write(&tmp, sealed)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &target)
    }
}

/// Removes every sealed file in `dir`.
pub fn wipe(dir: &Path) -> io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_file() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// Collects a new file and hands it to the directory, sealed, when tantivy says it is done.
struct SealedWriter {
    path: PathBuf,
    data: Vec<u8>,
    directory: SealedDirectory,
    done: bool,
}

impl Write for SealedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.data.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl TerminatingWrite for SealedWriter {
    fn terminate_ref(&mut self, _: AntiCallToken) -> io::Result<()> {
        if self.done {
            return Ok(());
        }
        self.done = true;
        let data = std::mem::take(&mut self.data);
        self.directory.persist(&self.path, &data)?;
        self.directory.ram.atomic_write(&self.path, &data)
    }
}

impl Directory for SealedDirectory {
    fn get_file_handle(&self, path: &Path) -> Result<Arc<dyn FileHandle>, OpenReadError> {
        self.ram.get_file_handle(path)
    }

    fn delete(&self, path: &Path) -> Result<(), DeleteError> {
        if let Some(disk) = &self.disk {
            let _ = std::fs::remove_file(disk.dir.join(disk_name(path)));
        }
        self.ram.delete(path)
    }

    fn exists(&self, path: &Path) -> Result<bool, OpenReadError> {
        self.ram.exists(path)
    }

    fn open_write(&self, path: &Path) -> Result<WritePtr, OpenWriteError> {
        if is_lock(path) {
            return self.ram.open_write(path);
        }
        if self.ram.exists(path).unwrap_or(false) {
            return Err(OpenWriteError::FileAlreadyExists(path.to_path_buf()));
        }
        let writer = SealedWriter { path: path.to_path_buf(), data: Vec::new(), directory: self.clone(), done: false };
        Ok(io::BufWriter::new(Box::new(writer)))
    }

    fn atomic_read(&self, path: &Path) -> Result<Vec<u8>, OpenReadError> {
        self.ram.atomic_read(path)
    }

    fn atomic_write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        self.persist(path, data)?;
        self.ram.atomic_write(path, data)
    }

    fn sync_directory(&self) -> io::Result<()> {
        Ok(())
    }

    fn watch(&self, watch_callback: WatchCallback) -> Result<WatchHandle, TantivyError> {
        self.ram.watch(watch_callback)
    }
}
