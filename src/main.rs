#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use directories::UserDirs;
use serde::Serialize;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use toml_edit::{DocumentMut, Item, Table, value};
use url::Url;

mod provider_sync;

use provider_sync::ProviderSyncReport;

const PROVIDER_ID: &str = "custom";

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
            start_official_login
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
fn save_proxy_config(api_url: String, api_key: String) -> Result<ProviderSyncReport, String> {
    let codex_home = resolve_codex_home().map_err(display_error)?;
    install_proxy(&codex_home, &api_url, &api_key).map_err(display_error)
}

#[tauri::command]
fn start_official_login() -> Result<(), String> {
    launch_codex_login().map_err(display_error)
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
    let api_key = validate_api_key(api_key)?;
    let config_path = codex_home.join("config.toml");

    fs::create_dir_all(codex_home)?;
    let original = if config_path.exists() {
        Some(fs::read(&config_path)?)
    } else {
        None
    };
    let original_text = original
        .as_deref()
        .map(std::str::from_utf8)
        .transpose()
        .map_err(|error| format!("现有 config.toml 不是 UTF-8：{error}"))?
        .unwrap_or_default();
    let updated = build_config(original_text, &api_url, api_key)?;
    let report = provider_sync::apply_provider_config(
        codex_home,
        original.as_deref(),
        updated.as_bytes(),
        PROVIDER_ID,
    )?;
    verify_config(&config_path, &api_url)?;
    Ok(report)
}

fn read_proxy_config(codex_home: &Path) -> Result<ProxyConfig, Box<dyn Error>> {
    let config_path = codex_home.join("config.toml");
    if !config_path.is_file() {
        return Ok(ProxyConfig {
            api_url: String::new(),
            has_api_key: false,
        });
    }

    let content = fs::read_to_string(config_path)?;
    let document = content
        .parse::<DocumentMut>()
        .map_err(|error| format!("现有 config.toml 解析失败：{error}"))?;
    let provider = document
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(PROVIDER_ID))
        .and_then(Item::as_table);

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

fn build_config(original: &str, api_url: &str, api_key: &str) -> Result<String, Box<dyn Error>> {
    let mut document = if original.trim().is_empty() {
        DocumentMut::new()
    } else {
        original
            .parse::<DocumentMut>()
            .map_err(|error| format!("现有 config.toml 解析失败：{error}"))?
    };

    document["model_provider"] = value(PROVIDER_ID);
    if !document.contains_key("model_providers") {
        let mut providers = Table::new();
        providers.set_implicit(true);
        document["model_providers"] = Item::Table(providers);
    }
    let providers = document["model_providers"]
        .as_table_mut()
        .ok_or("config.toml 中的 model_providers 不是表")?;

    let mut provider = Table::new();
    provider["name"] = value("QuotaPlusPlus");
    provider["base_url"] = value(api_url);
    provider["wire_api"] = value("responses");
    provider["requires_openai_auth"] = value(true);
    provider["supports_websockets"] = value(false);
    provider["experimental_bearer_token"] = value(api_key);
    providers[PROVIDER_ID] = Item::Table(provider);

    Ok(document.to_string())
}

fn verify_config(config_path: &Path, expected_url: &str) -> Result<(), Box<dyn Error>> {
    let content = fs::read_to_string(config_path)?;
    let document = content.parse::<DocumentMut>()?;
    let provider = document
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(PROVIDER_ID))
        .and_then(Item::as_table)
        .ok_or("写入后的 custom 提供方不存在")?;
    let valid = document.get("model_provider").and_then(Item::as_str) == Some(PROVIDER_ID)
        && provider.get("base_url").and_then(Item::as_str) == Some(expected_url)
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

#[cfg(windows)]
fn launch_codex_login() -> Result<(), Box<dyn Error>> {
    Command::new("cmd")
        .args([
            "/C",
            "start",
            "QuotaPlusPlus Login",
            "cmd",
            "/K",
            "codex login --device-auth",
        ])
        .spawn()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn launch_codex_login() -> Result<(), Box<dyn Error>> {
    Command::new("osascript")
        .args([
            "-e",
            "tell application \"Terminal\" to do script \"codex login --device-auth\"",
            "-e",
            "tell application \"Terminal\" to activate",
        ])
        .spawn()?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn launch_codex_login() -> Result<(), Box<dyn Error>> {
    let terminals: [(&str, &[&str]); 4] = [
        ("x-terminal-emulator", &["-e", "codex login --device-auth"]),
        ("gnome-terminal", &["--", "codex", "login", "--device-auth"]),
        ("konsole", &["-e", "codex", "login", "--device-auth"]),
        ("xterm", &["-e", "codex login --device-auth"]),
    ];
    for (terminal, args) in terminals {
        if Command::new(terminal).args(args).spawn().is_ok() {
            return Ok(());
        }
    }
    Err("未找到可用的终端程序".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn config_preserves_unmanaged_settings_and_providers() {
        let original = r#"model = "gpt-test"

[desktop]
localeOverride = "zh-CN"

[model_providers.existing]
name = "Existing"
base_url = "https://existing.example"
"#;
        let updated =
            build_config(original, "https://proxy.example/v1", "secret").expect("build config");
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
    }

    #[test]
    fn install_never_touches_auth_file() {
        let directory = tempdir().expect("tempdir");
        let auth_path = directory.path().join("auth.json");
        let auth = b"opaque official credentials";
        fs::write(&auth_path, auth).expect("write auth fixture");
        fs::write(
            directory.path().join("config.toml"),
            "model = \"gpt-test\"\n",
        )
        .expect("write config");

        install_proxy(directory.path(), "https://proxy.example/v1/", "TEST_KEY")
            .expect("install proxy");

        assert_eq!(fs::read(auth_path).expect("read auth fixture"), auth);
        let loaded = read_proxy_config(directory.path()).expect("read proxy config");
        assert_eq!(loaded.api_url, "https://proxy.example/v1");
        assert!(loaded.has_api_key);
    }

    #[test]
    fn rejects_invalid_inputs() {
        assert!(normalize_api_url("file:///tmp/api").is_err());
        assert!(normalize_api_url("https://proxy.example/v1?x=1").is_err());
        assert!(validate_api_key("has whitespace").is_err());
    }
}
