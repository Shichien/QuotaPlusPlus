use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const PROFILE_DIR: &str = "qpp-profiles";
const CURRENT_FILE: &str = "current";
const GENERATIONS_DIR: &str = "generations";
const EMPTY_GENERATION: &str = "none";
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
        load_committed_pair(&self.official_dir())
    }

    pub fn load_official_config(&self) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        let directory = self.official_dir();
        if let Some(config) = read_optional(&directory.join("config.toml"))? {
            return Ok(Some(config));
        }
        Ok(load_committed_pair(&directory)?.map(|profile| profile.config))
    }

    pub fn save_official(&self, config: &[u8], auth: &[u8]) -> Result<(), Box<dyn Error>> {
        let directory = self.official_dir();
        create_private_dir(&directory)?;
        ensure_legacy_pair_committed(&directory)?;
        atomic_write_private(&directory.join("config.toml"), config)?;
        commit_pair(&directory, config, auth)
    }

    pub fn save_official_config(&self, config: &[u8]) -> Result<(), Box<dyn Error>> {
        let directory = self.official_dir();
        create_private_dir(&directory)?;
        ensure_legacy_pair_committed(&directory)?;
        atomic_write_private(&directory.join("config.toml"), config)?;
        verify_file(&directory.join("config.toml"), config)
    }

    pub fn discard_official_auth(&self) -> Result<(), Box<dyn Error>> {
        let directory = self.official_dir();
        create_private_dir(&directory)?;
        atomic_write_private(&directory.join(CURRENT_FILE), EMPTY_GENERATION.as_bytes())?;
        let generations = directory.join(GENERATIONS_DIR);
        if generations.is_dir() {
            prune_generations(&generations, &generations.join(EMPTY_GENERATION))?;
        }
        remove_optional_file(&directory.join("auth.json"))
    }

    pub fn load_custom_config(&self) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        let directory = self.custom_dir();
        if let Some(profile) = load_committed_pair(&directory)? {
            return Ok(Some(profile.config));
        }
        read_optional(&directory.join("config.toml"))
    }

    pub fn load_custom_auth(&self) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        let directory = self.custom_dir();
        if let Some(profile) = load_committed_pair(&directory)? {
            return Ok(Some(profile.auth));
        }
        read_optional(&directory.join("auth.json"))
    }

    pub fn save_custom_config(&self, config: &[u8]) -> Result<(), Box<dyn Error>> {
        let directory = self.custom_dir();
        create_private_dir(&directory)?;
        ensure_legacy_pair_committed(&directory)?;
        atomic_write_private(&directory.join("config.toml"), config)?;
        verify_file(&directory.join("config.toml"), config)
    }

    pub fn save_custom_auth(&self, auth: &[u8]) -> Result<(), Box<dyn Error>> {
        let directory = self.custom_dir();
        create_private_dir(&directory)?;
        ensure_legacy_pair_committed(&directory)?;
        atomic_write_private(&directory.join("auth.json"), auth)?;
        verify_file(&directory.join("auth.json"), auth)?;
        if let Some(config) = read_optional(&directory.join("config.toml"))? {
            commit_pair(&directory, &config, auth)?;
        }
        Ok(())
    }

    fn official_dir(&self) -> PathBuf {
        self.root.join("official")
    }

    fn custom_dir(&self) -> PathBuf {
        self.root.join("custom")
    }
}

fn load_committed_pair(directory: &Path) -> Result<Option<OfficialProfile>, Box<dyn Error>> {
    let current = read_optional(&directory.join(CURRENT_FILE))?;
    if let Some(current) = current {
        let generation = std::str::from_utf8(&current)
            .map_err(|error| format!("快照指针不是 UTF-8：{error}"))?
            .trim();
        if generation == EMPTY_GENERATION {
            return Ok(None);
        }
        validate_generation(generation)?;
        let root = directory.join(GENERATIONS_DIR).join(generation);
        let config = fs::read(root.join("config.toml"))
            .map_err(|error| format!("已提交快照缺少 config.toml（{generation}）：{error}"))?;
        let auth = fs::read(root.join("auth.json"))
            .map_err(|error| format!("已提交快照缺少 auth.json（{generation}）：{error}"))?;
        return Ok(Some(OfficialProfile { auth, config }));
    }

    let auth = read_optional(&directory.join("auth.json"))?;
    let config = read_optional(&directory.join("config.toml"))?;
    Ok(match (auth, config) {
        (Some(auth), Some(config)) => Some(OfficialProfile { auth, config }),
        _ => None,
    })
}

fn ensure_legacy_pair_committed(directory: &Path) -> Result<(), Box<dyn Error>> {
    if directory.join(CURRENT_FILE).is_file() {
        return Ok(());
    }
    let config = read_optional(&directory.join("config.toml"))?;
    let auth = read_optional(&directory.join("auth.json"))?;
    if let (Some(config), Some(auth)) = (config, auth) {
        commit_pair(directory, &config, &auth)?;
    }
    Ok(())
}

fn commit_pair(directory: &Path, config: &[u8], auth: &[u8]) -> Result<(), Box<dyn Error>> {
    create_private_dir(directory)?;
    let generations = directory.join(GENERATIONS_DIR);
    create_private_dir(&generations)?;
    let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let created_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let generation = format!("generation-{}-{created_at}-{sequence}", std::process::id());
    let staging = generations.join(format!(".{generation}.tmp"));
    let destination = generations.join(&generation);
    let result = (|| -> Result<(), Box<dyn Error>> {
        fs::create_dir(&staging)?;
        #[cfg(unix)]
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o700))?;
        write_new_private(&staging.join("config.toml"), config)?;
        write_new_private(&staging.join("auth.json"), auth)?;
        verify_file(&staging.join("config.toml"), config)?;
        verify_file(&staging.join("auth.json"), auth)?;
        sync_directory(&staging)?;
        fs::rename(&staging, &destination)?;
        sync_directory(&generations)?;
        atomic_write_private(&directory.join(CURRENT_FILE), generation.as_bytes())?;
        prune_generations(&generations, &destination)?;
        remove_optional_file(&directory.join("auth.json"))?;
        Ok(())
    })();
    if result.is_err() && staging.is_dir() {
        let _ = fs::remove_dir_all(staging);
    }
    result
}

fn prune_generations(generations: &Path, current: &Path) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(generations)? {
        let entry = entry?;
        let path = entry.path();
        if path == current {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(format!("快照代次目录包含符号链接：{}", path.display()).into());
        }
        if file_type.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    sync_directory(generations)
}

fn validate_generation(generation: &str) -> Result<(), Box<dyn Error>> {
    if generation.is_empty()
        || !generation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("快照指针中的代次名称无效".into());
    }
    Ok(())
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
    match fs::read(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("读取快照文件失败 {}：{error}", path.display()).into()),
    }
}

fn create_private_dir(path: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(path).map_err(|error| -> Box<dyn Error> {
        format!("创建快照目录失败 {}：{error}", path.display()).into()
    })?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(
        |error| -> Box<dyn Error> {
            format!("设置快照目录权限失败 {}：{error}", path.display()).into()
        },
    )?;
    Ok(())
}

fn remove_optional_file(path: &Path) -> Result<(), Box<dyn Error>> {
    match fs::remove_file(path) {
        Ok(()) => sync_directory(path.parent().ok_or("快照文件没有父目录")?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn write_new_private(path: &Path, content: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(content)?;
    file.sync_all()?;
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
        sync_directory(parent)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| format!("写入快照文件失败 {}：{error}", path.display()).into())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), Box<dyn Error>> {
    fs::File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(windows)]
fn sync_directory(_path: &Path) -> Result<(), Box<dyn Error>> {
    Ok(())
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
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

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
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
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
        store
            .save_custom_auth(b"{\"OPENAI_API_KEY\":\"fixture-key\"}")
            .expect("save custom auth");
        assert_eq!(
            store
                .load_custom_auth()
                .expect("load custom auth")
                .expect("custom auth"),
            b"{\"OPENAI_API_KEY\":\"fixture-key\"}"
        );
    }

    #[test]
    fn custom_profile_never_exposes_a_half_written_pair() {
        let directory = tempdir().expect("tempdir");
        let store = ProfileStore::new(directory.path());
        store
            .save_custom_config(b"base_url = \"https://first.example\"\n")
            .expect("save first config");
        store
            .save_custom_auth(b"{\"OPENAI_API_KEY\":\"first-key\"}")
            .expect("commit first pair");

        store
            .save_custom_config(b"base_url = \"https://second.example\"\n")
            .expect("stage second config");

        assert_eq!(
            store
                .load_custom_config()
                .expect("load committed config")
                .expect("committed config"),
            b"base_url = \"https://first.example\"\n"
        );
        assert_eq!(
            store
                .load_custom_auth()
                .expect("load committed auth")
                .expect("committed auth"),
            b"{\"OPENAI_API_KEY\":\"first-key\"}"
        );

        store
            .save_custom_auth(b"{\"OPENAI_API_KEY\":\"second-key\"}")
            .expect("commit second pair");
        assert_eq!(
            store
                .load_custom_config()
                .expect("load second config")
                .expect("second config"),
            b"base_url = \"https://second.example\"\n"
        );
        assert_eq!(
            store
                .load_custom_auth()
                .expect("load second auth")
                .expect("second auth"),
            b"{\"OPENAI_API_KEY\":\"second-key\"}"
        );
        let generations = directory
            .path()
            .join(PROFILE_DIR)
            .join("custom")
            .join(GENERATIONS_DIR);
        let entries = fs::read_dir(generations)
            .expect("read generations")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect generations");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].file_type().expect("generation type").is_dir());
        assert!(
            !directory
                .path()
                .join(PROFILE_DIR)
                .join("custom/auth.json")
                .exists()
        );
    }

    #[test]
    fn discarding_official_auth_invalidates_the_committed_pair() {
        let directory = tempdir().expect("tempdir");
        let store = ProfileStore::new(directory.path());
        store
            .save_official(b"model = \"official\"\n", b"{\"tokens\":{}}")
            .expect("save official pair");

        store
            .discard_official_auth()
            .expect("discard official auth");

        assert!(store.load_official().expect("load official").is_none());
        assert_eq!(
            store
                .load_official_config()
                .expect("load retained config")
                .expect("retained config"),
            b"model = \"official\"\n"
        );
        assert_eq!(
            fs::read_dir(
                directory
                    .path()
                    .join(PROFILE_DIR)
                    .join("official")
                    .join(GENERATIONS_DIR)
            )
            .expect("read discarded generations")
            .count(),
            0
        );
    }
}
