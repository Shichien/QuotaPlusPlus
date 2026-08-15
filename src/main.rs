#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use directories::UserDirs;
use serde::Serialize;
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
    let updated = build_custom_config(original_text, &api_url, api_key)?;

    activate_custom(
        codex_home,
        original.as_deref(),
        updated.as_bytes(),
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
    profiles: &ProfileStore,
) -> Result<ProviderSyncReport, Box<dyn Error>> {
    profiles.save_custom_config(updated_config)?;
    let report = provider_sync::apply_provider_state(
        codex_home,
        original_config,
        updated_config,
        PROVIDER_ID,
        AuthUpdate::Remove,
    )?;
    verify_custom_config(&codex_home.join("config.toml"))?;
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
        has_api_key: provider
            .and_then(|table| table.get("experimental_bearer_token"))
            .and_then(Item::as_str)
            .is_some_and(|key| !key.is_empty()),
    })
}

fn read_stored_api_key(codex_home: &Path) -> Result<Option<String>, Box<dyn Error>> {
    let content = load_custom_source_config(codex_home)?;
    let Some(content) = content else {
        return Ok(None);
    };
    let content =
        std::str::from_utf8(&content).map_err(|error| format!("第三方配置不是 UTF-8：{error}"))?;
    let document = parse_config(content)?;
    Ok(custom_provider(&document)
        .and_then(|provider| provider.get("experimental_bearer_token"))
        .and_then(Item::as_str)
        .filter(|key| !key.is_empty())
        .map(str::to_string))
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

fn build_custom_config(
    original: &str,
    api_url: &str,
    api_key: &str,
) -> Result<String, Box<dyn Error>> {
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
    provider["experimental_bearer_token"] = value(api_key);
    Ok(document.to_string())
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

fn verify_custom_config(config_path: &Path) -> Result<(), Box<dyn Error>> {
    let content = fs::read_to_string(config_path)?;
    let document = parse_config(&content)?;
    let provider = custom_provider(&document).ok_or("写入后的 custom 提供方不存在")?;
    let valid = document.get("model_provider").and_then(Item::as_str) == Some(PROVIDER_ID)
        && provider.get("base_url").and_then(Item::as_str).is_some()
        && provider.get("wire_api").and_then(Item::as_str) == Some("responses")
        && provider.get("requires_openai_auth").and_then(Item::as_bool) == Some(true)
        && provider
            .get("experimental_bearer_token")
            .and_then(Item::as_str)
            .is_some_and(|key| !key.is_empty());
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
"#;
        let updated = build_custom_config(original, "https://proxy.example/v1", "secret")
            .expect("build config");
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
        assert_eq!(
            document["model_providers"]["custom"]["experimental_bearer_token"].as_str(),
            Some("secret")
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
            "fixture-key",
        )
        .expect("build custom");

        let custom_report = activate_custom(
            codex_home,
            Some(official_config),
            custom.as_bytes(),
            &profiles,
        )
        .expect("activate custom");
        assert!(!codex_home.join("auth.json").exists());
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
        let custom = build_custom_config(
            "model = \"custom\"\n",
            "https://proxy.example/v1",
            "fixture-key",
        )
        .expect("build custom");
        ProfileStore::new(directory.path())
            .save_custom_config(custom.as_bytes())
            .expect("save custom profile");

        let loaded = read_proxy_config(directory.path()).expect("read proxy config");
        assert_eq!(loaded.api_url, "https://proxy.example/v1");
        assert!(loaded.has_api_key);
        assert_eq!(
            read_stored_api_key(directory.path()).expect("read stored key"),
            Some("fixture-key".to_string())
        );
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
