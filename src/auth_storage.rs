use age::scrypt::Identity as ScryptIdentity;
use age::secrecy::SecretString;
use keyring::Entry;
use keyring::Error as KeyringError;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::Path;
use toml_edit::{DocumentMut, Item, value};

const KEYRING_SERVICE: &str = "Codex Auth";
const SECRETS_KEYRING_SERVICE: &str = "codex";
const CODEX_AUTH_SECRET_KEY: &str = "global/CODEX_AUTH";
const SECRETS_VERSION: u8 = 1;

#[derive(Deserialize)]
struct SecretsFile {
    version: u8,
    secrets: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum CredentialsStoreMode {
    #[default]
    File,
    Keyring,
    Auto,
    Ephemeral,
}

#[derive(Debug, Default)]
pub struct AuthDiscovery {
    pub candidates: Vec<Vec<u8>>,
    pub errors: Vec<String>,
}

impl AuthDiscovery {
    fn add_candidate(&mut self, candidate: Option<Vec<u8>>) {
        let Some(candidate) = candidate else {
            return;
        };
        if !self
            .candidates
            .iter()
            .any(|existing| existing == &candidate)
        {
            self.candidates.push(candidate);
        }
    }

    fn add_result(
        &mut self,
        source: &str,
        result: Result<Option<Vec<u8>>, Box<dyn Error>>,
        required: bool,
    ) {
        match result {
            Ok(candidate) => self.add_candidate(candidate),
            Err(error) if required => self.errors.push(format!("读取{source}失败：{error}")),
            Err(_) => {}
        }
    }
}

trait KeyringReader {
    fn load(&self, service: &str, account: &str) -> Result<Option<String>, Box<dyn Error>>;
}

struct SystemKeyring;

impl KeyringReader for SystemKeyring {
    fn load(&self, service: &str, account: &str) -> Result<Option<String>, Box<dyn Error>> {
        let entry = Entry::new(service, account)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

/// Finds persistent Codex credentials across every backend QPP can adopt.
pub fn discover_official_auth(
    codex_home: &Path,
    config: &[u8],
) -> Result<AuthDiscovery, Box<dyn Error>> {
    discover_official_auth_with(codex_home, config, &SystemKeyring)
}

/// QPP owns two file snapshots, so managed official sessions use auth.json after capture.
pub fn use_file_credentials(config: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let text = std::str::from_utf8(config)
        .map_err(|error| format!("官方 config.toml 不是 UTF-8：{error}"))?;
    let mut document = parse_config(text)?;
    document["cli_auth_credentials_store"] = value("file");
    Ok(document.to_string().into_bytes())
}

fn discover_official_auth_with(
    codex_home: &Path,
    config: &[u8],
    keyring: &dyn KeyringReader,
) -> Result<AuthDiscovery, Box<dyn Error>> {
    let mode = credentials_store_mode(config)?;
    let encrypted_first = secret_auth_storage_enabled(config)?;
    let direct_required = mode == CredentialsStoreMode::Keyring && !encrypted_first;
    let encrypted_required = (mode == CredentialsStoreMode::Keyring && encrypted_first)
        || codex_home.join("secrets").join("codex_auth.age").is_file();
    let mut discovery = AuthDiscovery::default();

    discovery.add_result(
        "auth.json",
        read_optional(&codex_home.join("auth.json")),
        true,
    );
    if encrypted_first {
        discovery.add_result(
            "Codex 加密凭据",
            load_encrypted_auth(codex_home, keyring),
            encrypted_required,
        );
        discovery.add_result(
            "系统钥匙串",
            load_direct_keyring_auth(codex_home, keyring),
            direct_required,
        );
    } else {
        discovery.add_result(
            "系统钥匙串",
            load_direct_keyring_auth(codex_home, keyring),
            direct_required,
        );
        discovery.add_result(
            "Codex 加密凭据",
            load_encrypted_auth(codex_home, keyring),
            encrypted_required,
        );
    }

    Ok(discovery)
}

fn load_direct_keyring_auth(
    codex_home: &Path,
    keyring: &dyn KeyringReader,
) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
    let account = compute_store_key(codex_home);
    Ok(keyring
        .load(KEYRING_SERVICE, &account)?
        .map(String::into_bytes))
}

fn load_encrypted_auth(
    codex_home: &Path,
    keyring: &dyn KeyringReader,
) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
    let path = codex_home.join("secrets").join("codex_auth.age");
    if !path.is_file() {
        return Ok(None);
    }
    let account = compute_secrets_key(codex_home);
    let Some(passphrase) = keyring.load(SECRETS_KEYRING_SERVICE, &account)? else {
        return Ok(None);
    };
    let ciphertext = fs::read(&path)?;
    let identity = ScryptIdentity::new(SecretString::from(passphrase));
    let plaintext = age::decrypt(&identity, &ciphertext)
        .map_err(|error| format!("官方加密凭据解密失败：{error}"))?;
    let document: SecretsFile = serde_json::from_slice(&plaintext)
        .map_err(|error| format!("官方加密凭据解析失败：{error}"))?;
    if document.version > SECRETS_VERSION {
        return Err(format!(
            "官方加密凭据版本 {} 高于 QPP 支持的版本 {SECRETS_VERSION}",
            document.version
        )
        .into());
    }
    Ok(document
        .secrets
        .get(CODEX_AUTH_SECRET_KEY)
        .cloned()
        .map(String::into_bytes))
}

fn credentials_store_mode(config: &[u8]) -> Result<CredentialsStoreMode, Box<dyn Error>> {
    let document = parse_config_bytes(config)?;
    match document
        .get("cli_auth_credentials_store")
        .and_then(Item::as_str)
        .unwrap_or("file")
    {
        "file" => Ok(CredentialsStoreMode::File),
        "keyring" => Ok(CredentialsStoreMode::Keyring),
        "auto" => Ok(CredentialsStoreMode::Auto),
        "ephemeral" => Ok(CredentialsStoreMode::Ephemeral),
        mode => Err(format!("不支持的 cli_auth_credentials_store：{mode}").into()),
    }
}

fn secret_auth_storage_enabled(config: &[u8]) -> Result<bool, Box<dyn Error>> {
    let document = parse_config_bytes(config)?;
    Ok(document
        .get("features")
        .and_then(Item::as_table)
        .and_then(|features| features.get("secret_auth_storage"))
        .and_then(Item::as_bool)
        .unwrap_or(cfg!(windows)))
}

fn parse_config_bytes(config: &[u8]) -> Result<DocumentMut, Box<dyn Error>> {
    let text = std::str::from_utf8(config)
        .map_err(|error| format!("官方 config.toml 不是 UTF-8：{error}"))?;
    parse_config(text)
}

fn parse_config(text: &str) -> Result<DocumentMut, Box<dyn Error>> {
    if text.trim().is_empty() {
        Ok(DocumentMut::new())
    } else {
        text.parse::<DocumentMut>()
            .map_err(|error| format!("官方 config.toml 解析失败：{error}").into())
    }
}

fn compute_store_key(codex_home: &Path) -> String {
    compute_scoped_key(codex_home, "cli")
}

fn compute_secrets_key(codex_home: &Path) -> String {
    compute_scoped_key(codex_home, "secrets")
}

fn compute_scoped_key(codex_home: &Path, prefix: &str) -> String {
    let canonical = codex_home
        .canonicalize()
        .unwrap_or_else(|_| codex_home.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let hex = format!("{digest:x}");
    format!("{prefix}|{}", &hex[..16])
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
    match fs::read(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io;
    use tempfile::tempdir;

    #[derive(Default)]
    struct MockKeyring {
        direct_value: Option<String>,
        secrets_value: Option<String>,
        error: bool,
        calls: RefCell<Vec<(String, String)>>,
    }

    impl KeyringReader for MockKeyring {
        fn load(&self, service: &str, account: &str) -> Result<Option<String>, Box<dyn Error>> {
            self.calls
                .borrow_mut()
                .push((service.to_string(), account.to_string()));
            if self.error {
                return Err(io::Error::other("fixture keyring error").into());
            }
            Ok(match service {
                KEYRING_SERVICE => self.direct_value.clone(),
                SECRETS_KEYRING_SERVICE => self.secrets_value.clone(),
                _ => None,
            })
        }
    }

    #[test]
    fn missing_file_means_no_official_login() {
        let home = tempdir().expect("tempdir");
        let discovery = discover_official_auth_with(home.path(), b"", &MockKeyring::default())
            .expect("discover auth");
        assert!(discovery.candidates.is_empty());
        assert!(discovery.errors.is_empty());
    }

    #[test]
    fn keyring_mode_uses_upstream_service_and_scoped_account() {
        let home = tempdir().expect("tempdir");
        let keyring = MockKeyring {
            direct_value: Some("{\"tokens\":{}}".to_string()),
            ..Default::default()
        };

        let discovery = discover_official_auth_with(
            home.path(),
            b"cli_auth_credentials_store = \"keyring\"\n[features]\nsecret_auth_storage = false\n",
            &keyring,
        )
        .expect("discover keyring auth");

        assert_eq!(discovery.candidates, vec![b"{\"tokens\":{}}".to_vec()]);
        let calls = keyring.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, KEYRING_SERVICE);
        assert_eq!(calls[0].1, compute_store_key(home.path()));
    }

    #[test]
    fn auto_discovers_keyring_and_file_and_tolerates_optional_keyring_failure() {
        let home = tempdir().expect("tempdir");
        fs::write(home.path().join("auth.json"), b"file-auth").expect("write auth");
        let keyring = MockKeyring {
            direct_value: Some("keyring-auth".to_string()),
            ..Default::default()
        };
        assert_eq!(
            discover_official_auth_with(
                home.path(),
                b"cli_auth_credentials_store = \"auto\"\n[features]\nsecret_auth_storage = false\n",
                &keyring,
            )
            .expect("discover auto")
            .candidates,
            vec![b"file-auth".to_vec(), b"keyring-auth".to_vec()]
        );

        let failed = MockKeyring {
            error: true,
            ..Default::default()
        };
        let discovery = discover_official_auth_with(
            home.path(),
            b"cli_auth_credentials_store = \"auto\"\n[features]\nsecret_auth_storage = false\n",
            &failed,
        );
        let discovery = discovery.expect("discover file fallback");
        assert_eq!(discovery.candidates, vec![b"file-auth".to_vec()]);
        assert!(discovery.errors.is_empty());
    }

    #[test]
    fn managed_official_config_preserves_settings_and_uses_file() {
        let original = b"model = \"gpt-fixture\"\ncli_auth_credentials_store = \"keyring\"\n[desktop]\nlocaleOverride = \"zh-CN\"\n";
        let updated = use_file_credentials(original).expect("update config");
        let document = parse_config_bytes(&updated).expect("parse updated");
        assert_eq!(document["model"].as_str(), Some("gpt-fixture"));
        assert_eq!(
            document["cli_auth_credentials_store"].as_str(),
            Some("file")
        );
        assert_eq!(
            document["desktop"]["localeOverride"].as_str(),
            Some("zh-CN")
        );
    }

    #[test]
    fn secret_auth_storage_reads_encrypted_official_auth() {
        use age::scrypt::Recipient as ScryptRecipient;

        let home = tempdir().expect("tempdir");
        let secrets_dir = home.path().join("secrets");
        fs::create_dir(&secrets_dir).expect("create secrets dir");
        let passphrase = "fixture-encryption-passphrase";
        let official_auth = "{\"auth_mode\":\"chatgpt\",\"tokens\":{}}";
        let plaintext = serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "secrets": {
                CODEX_AUTH_SECRET_KEY: official_auth,
            }
        }))
        .expect("serialize secrets");
        let recipient = ScryptRecipient::new(SecretString::from(passphrase.to_string()));
        let ciphertext = age::encrypt(&recipient, &plaintext).expect("encrypt secrets");
        fs::write(secrets_dir.join("codex_auth.age"), ciphertext).expect("write secrets");
        let keyring = MockKeyring {
            secrets_value: Some(passphrase.to_string()),
            ..Default::default()
        };
        let discovery = discover_official_auth_with(
            home.path(),
            b"cli_auth_credentials_store = \"keyring\"\n[features]\nsecret_auth_storage = true\n",
            &keyring,
        )
        .expect("discover auth");
        assert_eq!(
            discovery.candidates,
            vec![official_auth.as_bytes().to_vec()]
        );
        let calls = keyring.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, SECRETS_KEYRING_SERVICE);
        assert_eq!(calls[0].1, compute_secrets_key(home.path()));
        assert_eq!(calls[1].0, KEYRING_SERVICE);
    }

    #[test]
    fn discovery_continues_past_custom_auth_json_to_official_keyring() {
        let home = tempdir().expect("tempdir");
        let custom_auth = b"{\"OPENAI_API_KEY\":\"fixture-key\"}";
        let official_auth =
            "{\"auth_mode\":\"chatgpt\",\"tokens\":{\"refresh_token\":\"refresh\"}}";
        fs::write(home.path().join("auth.json"), custom_auth).expect("write custom auth");
        let keyring = MockKeyring {
            direct_value: Some(official_auth.to_string()),
            ..Default::default()
        };

        let discovery = discover_official_auth_with(
            home.path(),
            b"cli_auth_credentials_store = \"file\"\n",
            &keyring,
        )
        .expect("discover credentials");

        assert_eq!(
            discovery.candidates,
            vec![custom_auth.to_vec(), official_auth.as_bytes().to_vec()]
        );
        assert!(discovery.errors.is_empty());
    }

    #[test]
    fn discovery_deduplicates_credentials_shared_by_file_and_keyring() {
        let home = tempdir().expect("tempdir");
        let auth = "{\"auth_mode\":\"chatgpt\",\"tokens\":{\"refresh_token\":\"refresh\"}}";
        fs::write(home.path().join("auth.json"), auth).expect("write auth");
        let keyring = MockKeyring {
            direct_value: Some(auth.to_string()),
            ..Default::default()
        };

        let discovery =
            discover_official_auth_with(home.path(), b"", &keyring).expect("discover credentials");

        assert_eq!(discovery.candidates, vec![auth.as_bytes().to_vec()]);
    }
}
