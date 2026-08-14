use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const PROFILE_DIR: &str = "qpp-profiles";
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

pub struct OfficialProfile {
    pub auth: Vec<u8>,
    pub config: Vec<u8>,
}

pub struct ProfileStore {
    root: PathBuf,
}

impl ProfileStore {
    pub fn new(codex_home: &Path) -> Self {
        Self {
            root: codex_home.join(PROFILE_DIR),
        }
    }

    pub fn load_official(&self) -> Result<Option<OfficialProfile>, Box<dyn Error>> {
        let auth = read_optional(&self.official_dir().join("auth.json"))?;
        let config = read_optional(&self.official_dir().join("config.toml"))?;
        Ok(match (auth, config) {
            (Some(auth), Some(config)) => Some(OfficialProfile { auth, config }),
            _ => None,
        })
    }

    pub fn load_official_config(&self) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        read_optional(&self.official_dir().join("config.toml"))
    }

    pub fn save_official(&self, config: &[u8], auth: &[u8]) -> Result<(), Box<dyn Error>> {
        let directory = self.official_dir();
        create_private_dir(&directory)?;
        atomic_write_private(&directory.join("config.toml"), config)?;
        atomic_write_private(&directory.join("auth.json"), auth)?;
        verify_file(&directory.join("config.toml"), config)?;
        verify_file(&directory.join("auth.json"), auth)?;
        Ok(())
    }

    pub fn save_official_config(&self, config: &[u8]) -> Result<(), Box<dyn Error>> {
        let directory = self.official_dir();
        create_private_dir(&directory)?;
        atomic_write_private(&directory.join("config.toml"), config)?;
        verify_file(&directory.join("config.toml"), config)
    }

    pub fn discard_official_auth(&self) -> Result<(), Box<dyn Error>> {
        let path = self.official_dir().join("auth.json");
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn load_custom_config(&self) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        read_optional(&self.custom_dir().join("config.toml"))
    }

    pub fn save_custom_config(&self, config: &[u8]) -> Result<(), Box<dyn Error>> {
        let directory = self.custom_dir();
        create_private_dir(&directory)?;
        atomic_write_private(&directory.join("config.toml"), config)?;
        verify_file(&directory.join("config.toml"), config)
    }

    fn official_dir(&self) -> PathBuf {
        self.root.join("official")
    }

    fn custom_dir(&self) -> PathBuf {
        self.root.join("custom")
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
    match fs::read(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn create_private_dir(path: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn atomic_write_private(path: &Path, content: &[u8]) -> Result<(), Box<dyn Error>> {
    let parent = path.parent().ok_or("快照文件没有父目录")?;
    create_private_dir(parent)?;
    let temporary = parent.join(format!(
        ".{}.qpp-{}-{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("profile"),
        std::process::id(),
        NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<(), Box<dyn Error>> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary)?;
        file.write_all(content)?;
        file.sync_all()?;
        replace_file(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn verify_file(path: &Path, expected: &[u8]) -> Result<(), Box<dyn Error>> {
    if fs::read(path)? != expected {
        return Err(format!("快照写入验证失败：{}", path.display()).into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    fs::rename(source, destination)?;
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MoveFileExW};

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn stores_official_pair_and_custom_config_separately() {
        let directory = tempdir().expect("tempdir");
        let store = ProfileStore::new(directory.path());
        store
            .save_official(b"model = \"official\"\n", b"{\"tokens\":{}}")
            .expect("save official");
        store
            .save_custom_config(b"model_provider = \"custom\"\n")
            .expect("save custom");

        let official = store
            .load_official()
            .expect("load official")
            .expect("official profile");
        assert_eq!(official.config, b"model = \"official\"\n");
        assert_eq!(official.auth, b"{\"tokens\":{}}");
        assert_eq!(
            store
                .load_custom_config()
                .expect("load custom")
                .expect("custom config"),
            b"model_provider = \"custom\"\n"
        );
    }
}
