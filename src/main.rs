#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use directories::UserDirs;
use serde::Serialize;
use serde_json::{Value, json};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, TryLockError};
use toml_edit::{DocumentMut, Item, Table, value};
use url::Url;

mod oauth;
mod profiles;
mod provider_sync;

use oauth::AuthHealth;
use profiles::ProfileStore;
use provider_sync::{AuthUpdate, ProviderSyncReport};

const PROVIDER_ID: &str = "custom";
const OFFICIAL_PROVIDER_ID: &str = "openai";
static APP_OPERATION: Mutex<()> = Mutex::new(());

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProxyConfig {
    api_url: String,
    has_api_key: bool,
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            load_proxy_config,
            save_proxy_config,
            start_official_login,
            cancel_official_login
        ])
        .run(tauri::generate_context!())
        .expect("failed to run QuotaPlusPlus");
}

#[tauri::command]
fn load_proxy_config() -> Result<ProxyConfig, String> {
    let codex_home = resolve_codex_home().map_err(display_error)?;
    read_proxy_config(&codex_home).map_err(display_error)
}

#[tauri::command]
async fn save_proxy_config(api_url: String, api_key: String) -> Result<ProviderSyncReport, String> {
    let codex_home = resolve_codex_home().map_err(display_error)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = acquire_app_operation().map_err(display_error)?;
        install_proxy(&codex_home, &api_url, &api_key).map_err(display_error)
    })
    .await
    .map_err(|error| format!("API 配置任务异常结束：{error}"))?
}

#[tauri::command]
async fn start_official_login() -> Result<ProviderSyncReport, String> {
    oauth::begin_login();
    let codex_home = resolve_codex_home().map_err(display_error)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = acquire_app_operation().map_err(display_error)?;
        switch_to_official(&codex_home).map_err(display_error)
    })
    .await
    .map_err(|error| format!("官方登录任务异常结束：{error}"))?
}

#[tauri::command]
fn cancel_official_login() {
    oauth::cancel_login();
}

fn acquire_app_operation() -> Result<MutexGuard<'static, ()>, Box<dyn Error>> {
    match APP_OPERATION.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::WouldBlock) => Err("另一个 QuotaPlusPlus 操作正在进行".into()),
        Err(TryLockError::Poisoned(_)) => Err("QuotaPlusPlus 操作锁状态异常，请重启程序".into()),
    }
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn resolve_codex_home() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(path) = std::env::var_os("CODEX_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let user_dirs = UserDirs::new().ok_or("未找到用户主目录")?;
    Ok(user_dirs.home_dir().join(".codex"))
}

fn install_proxy(
    codex_home: &Path,
    api_url: &str,
    api_key: &str,
) -> Result<ProviderSyncReport, Box<dyn Error>> {
    let api_url = normalize_api_url(api_url)?;
    fs::create_dir_all(codex_home)?;
    let stored_api_key = read_stored_api_key(codex_home)?;
    let api_key = if api_key.trim().is_empty() {
        stored_api_key.as_deref().ok_or("API Key 不能为空")?
    } else {
        validate_api_key(api_key)?
    };
    let custom_auth = build_custom_auth(api_key)?;

    let config_path = codex_home.join("config.toml");
    let original = read_optional_file(&config_path)?;
    let original_bytes = original.as_deref().unwrap_or_default();
    let original_text = std::str::from_utf8(original_bytes)
        .map_err(|error| format!("现有 config.toml 不是 UTF-8：{error}"))?;
    let profiles = ProfileStore::new(codex_home);

    if is_official_config(original_text)? {
        capture_official_profile(codex_home, &profiles, original_bytes)?;
    } else {
        profiles.save_custom_config(original_bytes)?;
    }
    let updated = build_custom_config(original_text, &api_url)?;

    activate_custom(
        codex_home,
        original.as_deref(),
        updated.as_bytes(),
        &custom_auth,
        &profiles,
    )
}

fn switch_to_official(codex_home: &Path) -> Result<ProviderSyncReport, Box<dyn Error>> {
    oauth::ensure_login_active()?;
    fs::create_dir_all(codex_home)?;
    let config_path = codex_home.join("config.toml");
    let original = read_optional_file(&config_path)?;
    let original_bytes = original.as_deref().unwrap_or_default();
    let original_text = std::str::from_utf8(original_bytes)
        .map_err(|error| format!("现有 config.toml 不是 UTF-8：{error}"))?;
    let active_is_official = is_official_config(original_text)?;
    let profiles = ProfileStore::new(codex_home);

    if !active_is_official {
        profiles.save_custom_config(original_bytes)?;
        if let Some(auth) = read_custom_auth_for_profile(codex_home, original_bytes)? {
            profiles.save_custom_auth(&auth)?;
        }
    }

    let (candidate_auth, mut official_config) = if active_is_official {
        (
            read_optional_file(&codex_home.join("auth.json"))?,
            original_bytes.to_vec(),
        )
    } else if let Some(profile) = profiles.load_official()? {
        (Some(profile.auth), profile.config)
    } else {
        let config = match profiles.load_official_config()? {
            Some(config) => config,
            None => build_official_config(original_text)?.into_bytes(),
        };
        (None, config)
    };

    let official_text = std::str::from_utf8(&official_config)
        .map_err(|error| format!("官方配置快照不是 UTF-8：{error}"))?;
    if !is_official_config(official_text)? {
        official_config = build_official_config(official_text)?.into_bytes();
    }

    let official_auth = match candidate_auth {
        Some(auth) => match oauth::refresh_auth(&auth)? {
            AuthHealth::Valid(refreshed) => refreshed,
            AuthHealth::Invalid => oauth::browser_login()?,
        },
        None => oauth::browser_login()?,
    };

    oauth::ensure_login_active()?;
    profiles.save_official(&official_config, &official_auth)?;
    activate_official(
        codex_home,
        original.as_deref(),
        &official_config,
        &official_auth,
    )
}

fn capture_official_profile(
    codex_home: &Path,
    profiles: &ProfileStore,
    config: &[u8],
) -> Result<(), Box<dyn Error>> {
    profiles.save_official_config(config)?;
    let Some(auth) = read_optional_file(&codex_home.join("auth.json"))? else {
        profiles.discard_official_auth()?;
        return Ok(());
    };
    match oauth::refresh_auth(&auth) {
        Ok(AuthHealth::Valid(refreshed)) => profiles.save_official(config, &refreshed),
        Ok(AuthHealth::Invalid) => profiles.discard_official_auth(),
        Err(_) => profiles.save_official(config, &auth),
    }
}

fn activate_custom(
    codex_home: &Path,
    original_config: Option<&[u8]>,
    updated_config: &[u8],
    custom_auth: &[u8],
    profiles: &ProfileStore,
) -> Result<ProviderSyncReport, Box<dyn Error>> {
    profiles.save_custom_config(updated_config)?;
    profiles.save_custom_auth(custom_auth)?;
    let report = provider_sync::apply_provider_state(
        codex_home,
        original_config,
        updated_config,
        PROVIDER_ID,
        AuthUpdate::Replace(custom_auth),
    )?;
    verify_custom_config(
        &codex_home.join("config.toml"),
        &codex_home.join("auth.json"),
    )?;
    Ok(report)
}

fn activate_official(
    codex_home: &Path,
    original_config: Option<&[u8]>,
    official_config: &[u8],
    official_auth: &[u8],
) -> Result<ProviderSyncReport, Box<dyn Error>> {
    let report = provider_sync::apply_provider_state(
        codex_home,
        original_config,
        official_config,
        OFFICIAL_PROVIDER_ID,
        AuthUpdate::Replace(official_auth),
    )?;
    if fs::read(codex_home.join("config.toml"))? != official_config {
        return Err("官方 config.toml 恢复后的字节验证失败".into());
    }
    if fs::read(codex_home.join("auth.json"))? != official_auth {
        return Err("官方 auth.json 恢复后的字节验证失败".into());
    }
    Ok(report)
}

fn read_proxy_config(codex_home: &Path) -> Result<ProxyConfig, Box<dyn Error>> {
    let content = load_custom_source_config(codex_home)?;
    let Some(content) = content else {
        return Ok(ProxyConfig {
            api_url: String::new(),
            has_api_key: false,
        });
    };
    let content =
        std::str::from_utf8(&content).map_err(|error| format!("第三方配置不是 UTF-8：{error}"))?;
    let document = content
        .parse::<DocumentMut>()
        .map_err(|error| format!("第三方 config.toml 解析失败：{error}"))?;
    let provider = custom_provider(&document);

    Ok(ProxyConfig {
        api_url: provider
            .and_then(|table| table.get("base_url"))
            .and_then(Item::as_str)
            .unwrap_or_default()
            .to_string(),
        has_api_key: read_stored_api_key(codex_home)?.is_some(),
    })
}

fn read_stored_api_key(codex_home: &Path) -> Result<Option<String>, Box<dyn Error>> {
    if let Some(auth) = load_custom_auth(codex_home)?
        && let Some(key) = api_key_from_auth(&auth)?
    {
        return Ok(Some(key));
    }

    let Some(content) = load_custom_source_config(codex_home)? else {
        return Ok(None);
    };
    read_legacy_api_key_from_config(&content)
}

fn load_custom_auth(codex_home: &Path) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
    let active_config = read_optional_file(&codex_home.join("config.toml"))?;
    if let Some(config) = active_config.as_deref()
        && config_selects_custom(config)?
        && let Some(auth) = read_optional_file(&codex_home.join("auth.json"))?
    {
        return Ok(Some(auth));
    }
    ProfileStore::new(codex_home).load_custom_auth()
}

fn api_key_from_auth(auth: &[u8]) -> Result<Option<String>, Box<dyn Error>> {
    let document: Value = match serde_json::from_slice(auth) {
        Ok(Value::Object(document)) => Value::Object(document),
        _ => return Ok(None),
    };
    if document
        .get("auth_mode")
        .and_then(Value::as_str)
        .is_some_and(|mode| mode != "apikey")
    {
        return Ok(None);
    }
    Ok(document
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string))
}

fn read_legacy_api_key_from_config(config: &[u8]) -> Result<Option<String>, Box<dyn Error>> {
    let content =
        std::str::from_utf8(config).map_err(|error| format!("第三方配置不是 UTF-8：{error}"))?;
    let document = parse_config(content)?;
    Ok(custom_provider(&document)
        .and_then(|provider| {
            provider
                .iter()
                .find(|(key, item)| is_bearer_token_field(key) && item.as_str().is_some())
                .and_then(|(_, item)| item.as_str())
        })
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string))
}

fn read_custom_auth_for_profile(
    codex_home: &Path,
    active_config: &[u8],
) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
    if let Some(auth) = read_optional_file(&codex_home.join("auth.json"))?
        && api_key_from_auth(&auth)?.is_some()
    {
        return Ok(Some(auth));
    }
    let Some(key) = read_legacy_api_key_from_config(active_config)? else {
        return Ok(None);
    };
    Ok(Some(build_custom_auth(&key)?))
}

fn load_custom_source_config(codex_home: &Path) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
    let active = read_optional_file(&codex_home.join("config.toml"))?;
    if let Some(content) = active.as_deref()
        && config_selects_custom(content)?
    {
        return Ok(active);
    }
    let profiles = ProfileStore::new(codex_home);
    if let Some(content) = profiles.load_custom_config()? {
        return Ok(Some(content));
    }
    match active {
        Some(content) if config_has_custom_provider(&content)? => Ok(Some(content)),
        _ => Ok(None),
    }
}

fn normalize_api_url(input: &str) -> Result<String, Box<dyn Error>> {
    let input = input.trim().trim_end_matches('/');
    let parsed = Url::parse(input).map_err(|error| format!("API URL 无效：{error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("API URL 只支持 http 或 https".into());
    }
    if parsed.host_str().is_none() || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("API URL 必须是没有查询参数和片段的基础地址".into());
    }
    Ok(input.to_string())
}

fn validate_api_key(input: &str) -> Result<&str, Box<dyn Error>> {
    let key = input.trim();
    if key.is_empty() {
        return Err("API Key 不能为空".into());
    }
    if key.chars().any(char::is_whitespace) {
        return Err("API Key 不能包含空白字符".into());
    }
    Ok(key)
}

fn build_custom_config(original: &str, api_url: &str) -> Result<String, Box<dyn Error>> {
    let mut document = parse_config(original)?;
    document["model_provider"] = value(PROVIDER_ID);
    if !document.contains_key("model_providers") {
        let mut providers = Table::new();
        providers.set_implicit(true);
        document["model_providers"] = Item::Table(providers);
    }
    let providers = document["model_providers"]
        .as_table_mut()
        .ok_or("config.toml 中的 model_providers 不是表")?;
    if !providers.contains_key(PROVIDER_ID) {
        providers[PROVIDER_ID] = Item::Table(Table::new());
    }
    let provider = providers[PROVIDER_ID]
        .as_table_mut()
        .ok_or("config.toml 中的 model_providers.custom 不是表")?;
    provider["name"] = value("QuotaPlusPlus");
    provider["base_url"] = value(api_url);
    provider["wire_api"] = value("responses");
    provider["requires_openai_auth"] = value(true);
    provider["supports_websockets"] = value(false);
    let credential_keys = provider
        .iter()
        .map(|(key, _)| key.to_string())
        .filter(|key| is_provider_credential_field(key))
        .collect::<Vec<_>>();
    for key in credential_keys {
        provider.remove(&key);
    }
    Ok(document.to_string())
}

fn is_provider_credential_field(key: &str) -> bool {
    let normalized = normalize_config_key(key);
    matches!(
        normalized.as_str(),
        "envkey" | "envkeyinstructions" | "auth" | "aws"
    ) || is_bearer_token_field(key)
}

fn is_bearer_token_field(key: &str) -> bool {
    let normalized = normalize_config_key(key);
    normalized.contains("bearer") && normalized.contains("token")
}

fn normalize_config_key(key: &str) -> String {
    key.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn build_custom_auth(api_key: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let api_key = validate_api_key(api_key)?;
    Ok(serde_json::to_vec_pretty(&json!({
        "OPENAI_API_KEY": api_key,
    }))?)
}

fn build_official_config(original: &str) -> Result<String, Box<dyn Error>> {
    let mut document = parse_config(original)?;
    document.remove("model_provider");
    Ok(document.to_string())
}

fn parse_config(content: &str) -> Result<DocumentMut, Box<dyn Error>> {
    if content.trim().is_empty() {
        Ok(DocumentMut::new())
    } else {
        content
            .parse::<DocumentMut>()
            .map_err(|error| format!("现有 config.toml 解析失败：{error}").into())
    }
}

fn is_official_config(content: &str) -> Result<bool, Box<dyn Error>> {
    let document = parse_config(content)?;
    Ok(document
        .get("model_provider")
        .and_then(Item::as_str)
        .is_none_or(|provider| provider == OFFICIAL_PROVIDER_ID))
}

fn config_has_custom_provider(content: &[u8]) -> Result<bool, Box<dyn Error>> {
    let content =
        std::str::from_utf8(content).map_err(|error| format!("config.toml 不是 UTF-8：{error}"))?;
    let document = parse_config(content)?;
    Ok(custom_provider(&document).is_some())
}

fn config_selects_custom(content: &[u8]) -> Result<bool, Box<dyn Error>> {
    let content =
        std::str::from_utf8(content).map_err(|error| format!("config.toml 不是 UTF-8：{error}"))?;
    let document = parse_config(content)?;
    Ok(document.get("model_provider").and_then(Item::as_str) == Some(PROVIDER_ID))
}

fn custom_provider(document: &DocumentMut) -> Option<&Table> {
    document
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(PROVIDER_ID))
        .and_then(Item::as_table)
}

fn verify_custom_config(config_path: &Path, auth_path: &Path) -> Result<(), Box<dyn Error>> {
    let content = fs::read_to_string(config_path)?;
    let document = parse_config(&content)?;
    let provider = custom_provider(&document).ok_or("写入后的 custom 提供方不存在")?;
    let auth: Value = serde_json::from_slice(&fs::read(auth_path)?)?;
    let auth_has_only_api_key = auth.as_object().is_some_and(|object| {
        object.len() == 1
            && object
                .get("OPENAI_API_KEY")
                .and_then(Value::as_str)
                .is_some_and(|key| !key.trim().is_empty())
    });
    let valid = document.get("model_provider").and_then(Item::as_str) == Some(PROVIDER_ID)
        && provider.get("base_url").and_then(Item::as_str).is_some()
        && provider.get("wire_api").and_then(Item::as_str) == Some("responses")
        && provider.get("requires_openai_auth").and_then(Item::as_bool) == Some(true)
        && !provider
            .iter()
            .any(|(key, _)| is_provider_credential_field(key))
        && auth_has_only_api_key;
    if !valid {
        return Err("配置写入后的验证未通过".into());
    }
    Ok(())
}

fn read_optional_file(path: &Path) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
    match fs::read(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tempfile::tempdir;

    #[test]
    fn custom_config_preserves_unmanaged_settings_and_providers() {
        let original = r#"model = "gpt-test"

[desktop]
localeOverride = "zh-CN"

[model_providers.existing]
name = "Existing"
base_url = "https://existing.example"

[model_providers.custom]
request_max_retries = 9
experimental_bearer_token = "legacy-one"
experimetal_bearer_token = "legacy-two"
env_key = "LEGACY_API_KEY"
"#;
        let updated =
            build_custom_config(original, "https://proxy.example/v1").expect("build config");
        let document = updated.parse::<DocumentMut>().expect("parse config");
        assert_eq!(document["model"].as_str(), Some("gpt-test"));
        assert_eq!(
            document["desktop"]["localeOverride"].as_str(),
            Some("zh-CN")
        );
        assert_eq!(
            document["model_providers"]["existing"]["name"].as_str(),
            Some("Existing")
        );
        assert_eq!(document["model_provider"].as_str(), Some("custom"));
        let custom_provider = document["model_providers"]["custom"]
            .as_table()
            .expect("custom provider table");
        assert!(
            !custom_provider
                .iter()
                .any(|(key, _)| is_provider_credential_field(key))
        );
        assert_eq!(
            document["model_providers"]["custom"]["request_max_retries"].as_integer(),
            Some(9)
        );
    }

    #[test]
    fn official_and_custom_round_trip_restores_exact_active_files() {
        let directory = tempdir().expect("tempdir");
        let codex_home = directory.path();
        let official_config = b"model = \"gpt-official\"\n[desktop]\nlocaleOverride = \"zh-CN\"\n";
        let official_auth =
            b"{\"auth_mode\":\"chatgpt\",\"tokens\":{\"refresh_token\":\"sensitive-refresh-token\"}}";
        fs::write(codex_home.join("config.toml"), official_config).expect("write config");
        fs::write(codex_home.join("auth.json"), official_auth).expect("write auth");
        let rollout = codex_home.join("sessions/rollout-fixture.jsonl");
        fs::create_dir_all(rollout.parent().expect("rollout parent")).expect("create sessions");
        fs::write(
            &rollout,
            "{\"type\":\"session_meta\",\"payload\":{\"model_provider\":\"openai\"}}\n{\"message\":\"preserved\"}\n",
        )
        .expect("write rollout");
        let profiles = ProfileStore::new(codex_home);
        profiles
            .save_official(official_config, official_auth)
            .expect("save official profile");
        let custom = build_custom_config(
            std::str::from_utf8(official_config).expect("official utf8"),
            "https://proxy.example/v1",
        )
        .expect("build custom");
        let custom_auth = build_custom_auth("fixture-key").expect("build custom auth");

        let custom_report = activate_custom(
            codex_home,
            Some(official_config),
            custom.as_bytes(),
            &custom_auth,
            &profiles,
        )
        .expect("activate custom");
        let active_custom_auth: Value =
            serde_json::from_slice(&fs::read(codex_home.join("auth.json")).expect("custom auth"))
                .expect("parse custom auth");
        assert_eq!(
            active_custom_auth.as_object().expect("auth object").len(),
            1
        );
        assert_eq!(active_custom_auth["OPENAI_API_KEY"], "fixture-key");
        let backup = Path::new(&custom_report.backup_path);
        assert_eq!(
            fs::read(backup.join("auth.json")).expect("backup auth"),
            official_auth
        );
        let manifest = fs::read_to_string(backup.join("manifest.json")).expect("read manifest");
        assert!(manifest.contains("\"authPresent\": true"));
        assert!(!manifest.contains("sensitive-refresh-token"));
        let custom_header: Value = serde_json::from_str(
            fs::read_to_string(&rollout)
                .expect("read custom rollout")
                .lines()
                .next()
                .expect("custom header"),
        )
        .expect("parse custom header");
        assert_eq!(custom_header["payload"]["model_provider"], "custom");

        let profile = profiles
            .load_official()
            .expect("load official")
            .expect("official profile");
        let active_custom = fs::read(codex_home.join("config.toml")).expect("read custom config");
        activate_official(
            codex_home,
            Some(&active_custom),
            &profile.config,
            &profile.auth,
        )
        .expect("activate official");

        assert_eq!(
            fs::read(codex_home.join("config.toml")).expect("restored config"),
            official_config
        );
        assert_eq!(
            fs::read(codex_home.join("auth.json")).expect("restored auth"),
            official_auth
        );
        let restored = fs::read_to_string(rollout).expect("read restored rollout");
        let restored_header: Value =
            serde_json::from_str(restored.lines().next().expect("restored header"))
                .expect("parse restored header");
        assert_eq!(restored_header["payload"]["model_provider"], "openai");
        assert!(restored.contains("preserved"));
    }

    #[test]
    fn proxy_config_loads_saved_custom_profile_while_official_is_active() {
        let directory = tempdir().expect("tempdir");
        fs::write(
            directory.path().join("config.toml"),
            "model = \"official\"\n[model_providers.custom]\nbase_url = \"https://stale.example/v1\"\nexperimental_bearer_token = \"stale-key\"\n",
        )
        .expect("write official config");
        let custom = build_custom_config("model = \"custom\"\n", "https://proxy.example/v1")
            .expect("build custom");
        let profiles = ProfileStore::new(directory.path());
        profiles
            .save_custom_config(custom.as_bytes())
            .expect("save custom profile");
        profiles
            .save_custom_auth(&build_custom_auth("fixture-key").expect("build custom auth"))
            .expect("save custom auth profile");

        let loaded = read_proxy_config(directory.path()).expect("read proxy config");
        assert_eq!(loaded.api_url, "https://proxy.example/v1");
        assert!(loaded.has_api_key);
        assert_eq!(
            read_stored_api_key(directory.path()).expect("read stored key"),
            Some("fixture-key".to_string())
        );
    }

    #[test]
    fn migrates_legacy_bearer_key_into_custom_auth_profile() {
        let directory = tempdir().expect("tempdir");
        let codex_home = directory.path();
        let legacy = "model_provider = \"custom\"\n\n[model_providers.custom]\nname = \"Legacy\"\nbase_url = \"https://proxy.example/v1\"\nexperimetal_bearer_token = \"legacy-key\"\n";
        fs::write(codex_home.join("config.toml"), legacy).expect("write legacy config");
        assert_eq!(
            read_stored_api_key(codex_home).expect("read legacy key"),
            Some("legacy-key".to_string())
        );
        let migrated = build_custom_auth(
            &read_stored_api_key(codex_home)
                .expect("read key")
                .expect("legacy key"),
        )
        .expect("build migrated auth");
        ProfileStore::new(codex_home)
            .save_custom_auth(&migrated)
            .expect("save migrated auth");
        let auth: Value = serde_json::from_slice(
            &ProfileStore::new(codex_home)
                .load_custom_auth()
                .expect("load migrated auth")
                .expect("migrated auth"),
        )
        .expect("parse migrated auth");
        assert_eq!(auth.as_object().expect("auth object").len(), 1);
        assert_eq!(auth["OPENAI_API_KEY"], "legacy-key");
    }

    #[test]
    fn custom_auth_has_standard_codex_api_key_shape() {
        let auth: Value =
            serde_json::from_slice(&build_custom_auth("fixture-key").expect("build custom auth"))
                .expect("parse custom auth");
        assert_eq!(
            auth,
            json!({
                "OPENAI_API_KEY": "fixture-key",
            })
        );
    }

    #[test]
    fn install_proxy_writes_api_key_to_active_auth_file() {
        let directory = tempdir().expect("tempdir");
        let codex_home = directory.path();
        fs::write(codex_home.join("config.toml"), "model = \"gpt-test\"\n").expect("write config");

        install_proxy(codex_home, "https://proxy.example/v1", "fixture-key")
            .expect("install proxy");

        let config = fs::read_to_string(codex_home.join("config.toml")).expect("read config");
        assert!(config.contains("model_provider = \"custom\""));
        assert!(!config.contains("bearer_token"));
        let auth: Value =
            serde_json::from_slice(&fs::read(codex_home.join("auth.json")).expect("read auth"))
                .expect("parse auth");
        assert_eq!(auth.as_object().expect("auth object").len(), 1);
        assert_eq!(auth["OPENAI_API_KEY"], "fixture-key");
    }

    #[test]
    fn install_proxy_migrates_legacy_bearer_field_when_key_input_is_empty() {
        let directory = tempdir().expect("tempdir");
        let codex_home = directory.path();
        fs::write(
            codex_home.join("config.toml"),
            "model_provider = \"custom\"\n\n[model_providers.custom]\nbase_url = \"https://old.example/v1\"\nexperimetal_bearer_token = \"legacy-key\"\n",
        )
        .expect("write legacy config");

        install_proxy(codex_home, "https://proxy.example/v1", "")
            .expect("migrate legacy proxy config");

        let config = fs::read_to_string(codex_home.join("config.toml")).expect("read config");
        let document = parse_config(&config).expect("parse migrated config");
        let provider = custom_provider(&document).expect("custom provider");
        assert!(
            !provider
                .iter()
                .any(|(key, _)| is_provider_credential_field(key))
        );
        let auth: Value =
            serde_json::from_slice(&fs::read(codex_home.join("auth.json")).expect("read auth"))
                .expect("parse auth");
        assert_eq!(auth, json!({"OPENAI_API_KEY": "legacy-key"}));
    }

    #[test]
    fn official_config_deactivates_any_third_party_provider() {
        let original = "model_provider = \"another\"\nmodel = \"gpt-test\"\n";
        let updated = build_official_config(original).expect("build official config");
        assert!(is_official_config(&updated).expect("classify official"));
        assert!(updated.contains("model = \"gpt-test\""));
        assert!(!updated.contains("model_provider ="));
    }

    #[test]
    fn rejects_invalid_inputs() {
        assert!(normalize_api_url("file:///tmp/api").is_err());
        assert!(normalize_api_url("https://proxy.example/v1?x=1").is_err());
        assert!(validate_api_key("has whitespace").is_err());
    }
}
