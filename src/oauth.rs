use base64::Engine;
use chrono::Utc;
use rand::RngCore;
use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tiny_http::{Request, Response, Server, StatusCode as TinyStatusCode};
use url::Url;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const ISSUER: &str = "https://auth.openai.com";
const CALLBACK_PORTS: [u16; 2] = [1455, 1457];
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const CALLBACK_POLL_INTERVAL: Duration = Duration::from_millis(100);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
static LOGIN_CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, PartialEq)]
pub enum AuthHealth {
    Valid(Vec<u8>),
    Invalid,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
}

struct PkceCodes {
    verifier: String,
    challenge: String,
}

enum Callback {
    Code(String),
    Rejected(String),
    StateMismatch,
    MissingCode,
}

pub fn refresh_auth(auth: &[u8]) -> Result<AuthHealth, Box<dyn Error>> {
    let client = http_client()?;
    refresh_auth_with(&client, &format!("{ISSUER}/oauth/token"), auth)
}

pub fn begin_login() {
    LOGIN_CANCELLED.store(false, Ordering::SeqCst);
}

pub fn cancel_login() {
    LOGIN_CANCELLED.store(true, Ordering::SeqCst);
}

pub fn ensure_login_active() -> Result<(), Box<dyn Error>> {
    ensure_not_cancelled(&LOGIN_CANCELLED)
}

pub fn browser_login() -> Result<Vec<u8>, Box<dyn Error>> {
    ensure_login_active()?;
    let (server, port) = bind_callback_server()?;
    let redirect_uri = format!("http://localhost:{port}/auth/callback");
    let pkce = generate_pkce();
    let state = random_urlsafe(32);
    let auth_url = build_authorize_url(&redirect_uri, &pkce.challenge, &state)?;

    ensure_login_active()?;
    webbrowser::open(auth_url.as_str()).map_err(|error| format!("打开登录页面失败：{error}"))?;

    let (request, code) =
        wait_for_authorization_code(&server, &state, LOGIN_TIMEOUT, &LOGIN_CANCELLED)?;
    if let Err(error) = ensure_login_active() {
        respond(request, 409, "Sign-in cancelled. Return to QuotaPlusPlus.")?;
        return Err(error);
    }

    let client = http_client()?;
    let result = exchange_code(
        &client,
        &format!("{ISSUER}/oauth/token"),
        &redirect_uri,
        &pkce.verifier,
        &code,
    );
    match result {
        Ok(auth) => {
            if let Err(error) = ensure_login_active() {
                respond(request, 409, "Sign-in cancelled. Return to QuotaPlusPlus.")?;
                return Err(error);
            }
            respond(request, 200, "Sign-in complete. You can close this page.")?;
            Ok(auth)
        }
        Err(error) => {
            respond(
                request,
                502,
                "Token exchange failed. Return to QuotaPlusPlus.",
            )?;
            Err(error)
        }
    }
}

fn wait_for_authorization_code(
    server: &Server,
    expected_state: &str,
    timeout: Duration,
    cancelled: &AtomicBool,
) -> Result<(Request, String), Box<dyn Error>> {
    let deadline = Instant::now() + timeout;
    loop {
        ensure_not_cancelled(cancelled)?;
        let now = Instant::now();
        if now >= deadline {
            return Err("官方登录等待超时，请重新点击官方登录".into());
        }
        let wait = (deadline - now).min(CALLBACK_POLL_INTERVAL);
        let request = server.recv_timeout(wait)?;
        if cancelled.load(Ordering::SeqCst) {
            if let Some(request) = request {
                respond(request, 409, "Sign-in cancelled. Return to QuotaPlusPlus.")?;
            }
            return Err("官方登录已取消".into());
        }
        let Some(request) = request else {
            continue;
        };
        let callback = parse_callback(request.url(), expected_state);
        match callback {
            Callback::StateMismatch => {
                respond(request, 400, "Invalid OAuth state")?;
            }
            Callback::MissingCode => {
                respond(request, 400, "Missing authorization code")?;
            }
            Callback::Rejected(reason) => {
                respond(request, 400, "OpenAI sign-in was not completed")?;
                return Err(format!("官方登录未完成：{reason}").into());
            }
            Callback::Code(code) => {
                return Ok((request, code));
            }
        }
    }
}

fn ensure_not_cancelled(cancelled: &AtomicBool) -> Result<(), Box<dyn Error>> {
    if cancelled.load(Ordering::SeqCst) {
        return Err("官方登录已取消".into());
    }
    Ok(())
}

fn http_client() -> Result<Client, Box<dyn Error>> {
    Ok(Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(concat!("QuotaPlusPlus/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

fn refresh_auth_with(
    client: &Client,
    token_url: &str,
    auth: &[u8],
) -> Result<AuthHealth, Box<dyn Error>> {
    let mut document: Value = match serde_json::from_slice(auth) {
        Ok(Value::Object(object)) => Value::Object(object),
        _ => return Ok(AuthHealth::Invalid),
    };
    let Some(refresh_token) = token_field(&document, "refresh_token") else {
        return Ok(AuthHealth::Invalid);
    };
    if refresh_token.trim().is_empty() {
        return Ok(AuthHealth::Invalid);
    }

    let response = client
        .post(token_url)
        .header("Content-Type", "application/json")
        .json(&json!({
            "client_id": CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
        }))
        .send()?;
    let status = response.status();
    let body = response.bytes()?;
    if !status.is_success() {
        if matches!(status, StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED) {
            return Ok(AuthHealth::Invalid);
        }
        return Err(format!("官方登录测活失败，令牌服务返回 {status}").into());
    }

    let refreshed: TokenResponse =
        serde_json::from_slice(&body).map_err(|error| format!("官方登录测活响应无效：{error}"))?;
    update_auth_document(&mut document, refreshed)?;
    Ok(AuthHealth::Valid(serde_json::to_vec_pretty(&document)?))
}

fn exchange_code(
    client: &Client,
    token_url: &str,
    redirect_uri: &str,
    verifier: &str,
    code: &str,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let response = client
        .post(token_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", CLIENT_ID),
            ("code_verifier", verifier),
        ])
        .send()?;
    let status = response.status();
    let body = response.bytes()?;
    if !status.is_success() {
        return Err(format!("官方登录令牌交换失败，令牌服务返回 {status}").into());
    }
    let tokens: TokenResponse =
        serde_json::from_slice(&body).map_err(|error| format!("官方登录令牌响应无效：{error}"))?;
    let id_token = required_response_token(tokens.id_token, "id_token")?;
    let access_token = required_response_token(tokens.access_token, "access_token")?;
    let refresh_token = required_response_token(tokens.refresh_token, "refresh_token")?;
    let account_id = account_id_from_jwt(&id_token).or_else(|| account_id_from_jwt(&access_token));
    let auth = json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": id_token,
            "access_token": access_token,
            "refresh_token": refresh_token,
            "account_id": account_id,
        },
        "last_refresh": Utc::now().to_rfc3339(),
    });
    Ok(serde_json::to_vec_pretty(&auth)?)
}

fn required_response_token(value: Option<String>, name: &str) -> Result<String, Box<dyn Error>> {
    value
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| format!("官方登录令牌响应缺少 {name}").into())
}

fn update_auth_document(
    document: &mut Value,
    refreshed: TokenResponse,
) -> Result<(), Box<dyn Error>> {
    let old_id = token_field(document, "id_token");
    let old_access = token_field(document, "access_token");
    let old_refresh = token_field(document, "refresh_token");
    let id_token = refreshed.id_token.or(old_id);
    let access_token = refreshed.access_token.or(old_access);
    let refresh_token = refreshed.refresh_token.or(old_refresh);
    let id_token = required_response_token(id_token, "id_token")?;
    let access_token = required_response_token(access_token, "access_token")?;
    let refresh_token = required_response_token(refresh_token, "refresh_token")?;
    let account_id = existing_account_id(document)
        .or_else(|| account_id_from_jwt(&id_token))
        .or_else(|| account_id_from_jwt(&access_token));

    let object = document.as_object_mut().ok_or("auth.json 顶层必须是对象")?;
    object.insert(
        "auth_mode".to_string(),
        Value::String("chatgpt".to_string()),
    );
    object.insert(
        "last_refresh".to_string(),
        Value::String(Utc::now().to_rfc3339()),
    );
    let tokens = object
        .entry("tokens")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or("auth.json 中的 tokens 必须是对象")?;
    tokens.insert("id_token".to_string(), Value::String(id_token.clone()));
    tokens.insert(
        "access_token".to_string(),
        Value::String(access_token.clone()),
    );
    tokens.insert(
        "refresh_token".to_string(),
        Value::String(refresh_token.clone()),
    );
    tokens.insert(
        "account_id".to_string(),
        account_id.clone().map(Value::String).unwrap_or(Value::Null),
    );

    // Older exported sessions used flat token fields. Keep those fields current when present.
    for (name, value) in [
        ("id_token", id_token),
        ("access_token", access_token),
        ("refresh_token", refresh_token),
    ] {
        if object.contains_key(name) {
            object.insert(name.to_string(), Value::String(value));
        }
    }
    if object.contains_key("chatgpt_account_id")
        && let Some(account_id) = account_id
    {
        object.insert("chatgpt_account_id".to_string(), Value::String(account_id));
    }
    Ok(())
}

fn token_field(document: &Value, name: &str) -> Option<String> {
    document
        .get("tokens")
        .and_then(|tokens| tokens.get(name))
        .or_else(|| document.get(name))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn existing_account_id(document: &Value) -> Option<String> {
    document
        .get("tokens")
        .and_then(|tokens| tokens.get("account_id"))
        .or_else(|| document.get("chatgpt_account_id"))
        .or_else(|| document.get("account_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn account_id_from_jwt(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .or_else(|| claims.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn bind_callback_server() -> Result<(Server, u16), Box<dyn Error>> {
    let mut errors = Vec::new();
    for port in CALLBACK_PORTS {
        match Server::http(format!("127.0.0.1:{port}")) {
            Ok(server) => return Ok((server, port)),
            Err(error) => errors.push(format!("{port}: {error}")),
        }
    }
    Err(format!("本地登录回调端口不可用：{}", errors.join("；")).into())
}

fn build_authorize_url(
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> Result<Url, Box<dyn Error>> {
    let mut url = Url::parse(&format!("{ISSUER}/oauth/authorize"))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair(
            "scope",
            "openid profile email offline_access api.connectors.read api.connectors.invoke",
        )
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", state)
        .append_pair("originator", "codex_cli_rs");
    Ok(url)
}

fn generate_pkce() -> PkceCodes {
    let verifier = random_urlsafe(64);
    let challenge = pkce_challenge(&verifier);
    PkceCodes {
        verifier,
        challenge,
    }
}

fn random_urlsafe(size: usize) -> String {
    let mut bytes = vec![0_u8; size];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn parse_callback(raw_url: &str, expected_state: &str) -> Callback {
    let Ok(url) = Url::parse(&format!("http://localhost{raw_url}")) else {
        return Callback::MissingCode;
    };
    if url.path() != "/auth/callback" {
        return Callback::MissingCode;
    }
    let params = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
    let state = params.get("state").map(String::as_str);
    if state != Some(expected_state) {
        return Callback::StateMismatch;
    }
    if let Some(error) = params.get("error") {
        return Callback::Rejected(error.to_string());
    }
    params
        .get("code")
        .filter(|code| !code.is_empty())
        .map(|code| Callback::Code(code.to_string()))
        .unwrap_or(Callback::MissingCode)
}

fn respond(request: Request, status: u16, body: &str) -> Result<(), Box<dyn Error>> {
    request.respond(Response::from_string(body).with_status_code(TinyStatusCode(status)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    fn jwt(account_id: &str) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{}");
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "https://api.openai.com/auth": {"chatgpt_account_id": account_id}
            }))
            .expect("serialize jwt payload"),
        );
        format!("{header}.{payload}.signature")
    }

    fn mock_token_server(status: u16, response: Value) -> (String, thread::JoinHandle<Value>) {
        let server = Server::http("127.0.0.1:0").expect("bind mock server");
        let address = server.server_addr().to_ip().expect("mock address");
        let url = format!("http://{address}/oauth/token");
        let handle = thread::spawn(move || {
            let mut request = server.recv().expect("receive refresh request");
            let mut body = String::new();
            request
                .as_reader()
                .read_to_string(&mut body)
                .expect("read refresh body");
            request
                .respond(
                    Response::from_string(response.to_string())
                        .with_status_code(TinyStatusCode(status))
                        .with_header(
                            "Content-Type: application/json"
                                .parse::<tiny_http::Header>()
                                .expect("content type"),
                        ),
                )
                .expect("respond refresh");
            serde_json::from_str(&body).expect("parse refresh request")
        });
        (url, handle)
    }

    fn mock_exchange_server(
        response: Value,
    ) -> (String, thread::JoinHandle<HashMap<String, String>>) {
        let server = Server::http("127.0.0.1:0").expect("bind exchange server");
        let address = server.server_addr().to_ip().expect("exchange address");
        let url = format!("http://{address}/oauth/token");
        let handle = thread::spawn(move || {
            let mut request = server.recv().expect("receive exchange request");
            let mut body = String::new();
            request
                .as_reader()
                .read_to_string(&mut body)
                .expect("read exchange body");
            request
                .respond(
                    Response::from_string(response.to_string()).with_header(
                        "Content-Type: application/json"
                            .parse::<tiny_http::Header>()
                            .expect("content type"),
                    ),
                )
                .expect("respond exchange");
            url::form_urlencoded::parse(body.as_bytes())
                .into_owned()
                .collect::<HashMap<_, _>>()
        });
        (url, handle)
    }

    #[test]
    fn refreshes_nested_tokens_and_keeps_rotated_refresh_token() {
        let old_id = jwt("account-one");
        let new_id = jwt("account-one");
        let auth = serde_json::to_vec(&json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {
                "id_token": old_id,
                "access_token": "old-access",
                "refresh_token": "old-refresh",
                "account_id": "account-one"
            }
        }))
        .expect("serialize auth");
        let (url, server) = mock_token_server(
            200,
            json!({
                "id_token": new_id,
                "access_token": "new-access",
                "refresh_token": "new-refresh"
            }),
        );
        let client = http_client().expect("http client");

        let AuthHealth::Valid(updated) =
            refresh_auth_with(&client, &url, &auth).expect("refresh auth")
        else {
            panic!("expected valid auth");
        };
        let updated: Value = serde_json::from_slice(&updated).expect("parse updated auth");
        assert_eq!(updated["tokens"]["access_token"], "new-access");
        assert_eq!(updated["tokens"]["refresh_token"], "new-refresh");
        let request = server.join().expect("join mock server");
        assert_eq!(request["grant_type"], "refresh_token");
        assert_eq!(request["client_id"], CLIENT_ID);
        assert_eq!(request["refresh_token"], "old-refresh");
    }

    #[test]
    fn exchanges_authorization_code_with_official_pkce_form() {
        let id_token = jwt("account-exchange");
        let (url, server) = mock_exchange_server(json!({
            "id_token": id_token,
            "access_token": "exchange-access",
            "refresh_token": "exchange-refresh"
        }));
        let client = http_client().expect("http client");

        let auth = exchange_code(
            &client,
            &url,
            "http://localhost:1455/auth/callback",
            "fixture-verifier",
            "fixture-code",
        )
        .expect("exchange code");
        let auth: Value = serde_json::from_slice(&auth).expect("parse exchanged auth");
        assert_eq!(auth["auth_mode"], "chatgpt");
        assert_eq!(auth["tokens"]["account_id"], "account-exchange");
        assert_eq!(auth["tokens"]["access_token"], "exchange-access");

        let form = server.join().expect("join exchange server");
        assert_eq!(
            form.get("grant_type").map(String::as_str),
            Some("authorization_code")
        );
        assert_eq!(form.get("client_id").map(String::as_str), Some(CLIENT_ID));
        assert_eq!(form.get("code").map(String::as_str), Some("fixture-code"));
        assert_eq!(
            form.get("code_verifier").map(String::as_str),
            Some("fixture-verifier")
        );
        assert_eq!(
            form.get("redirect_uri").map(String::as_str),
            Some("http://localhost:1455/auth/callback")
        );
    }

    #[test]
    fn accepts_flat_export_and_normalizes_it() {
        let id_token = jwt("account-flat");
        let auth = serde_json::to_vec(&json!({
            "id_token": id_token,
            "access_token": "flat-access",
            "refresh_token": "flat-refresh",
            "chatgpt_account_id": "account-flat"
        }))
        .expect("serialize auth");
        let (url, server) = mock_token_server(
            200,
            json!({"access_token": "new-access", "refresh_token": "new-refresh"}),
        );
        let client = http_client().expect("http client");

        let AuthHealth::Valid(updated) =
            refresh_auth_with(&client, &url, &auth).expect("refresh flat auth")
        else {
            panic!("expected valid auth");
        };
        let updated: Value = serde_json::from_slice(&updated).expect("parse updated auth");
        assert_eq!(updated["auth_mode"], "chatgpt");
        assert_eq!(updated["tokens"]["account_id"], "account-flat");
        assert_eq!(updated["access_token"], "new-access");
        server.join().expect("join mock server");
    }

    #[test]
    fn rejected_refresh_is_invalid_without_echoing_tokens() {
        let auth = serde_json::to_vec(&json!({
            "tokens": {
                "id_token": jwt("account-bad"),
                "access_token": "access-secret",
                "refresh_token": "refresh-secret"
            }
        }))
        .expect("serialize auth");
        let (url, server) = mock_token_server(
            401,
            json!({"error": "invalid_grant", "detail": "refresh-secret"}),
        );
        let client = http_client().expect("http client");

        assert_eq!(
            refresh_auth_with(&client, &url, &auth).expect("classify auth"),
            AuthHealth::Invalid
        );
        server.join().expect("join mock server");
    }

    #[test]
    fn transient_refresh_error_does_not_echo_credentials() {
        let auth = serde_json::to_vec(&json!({
            "tokens": {
                "id_token": jwt("account-transient"),
                "access_token": "access-private-value",
                "refresh_token": "refresh-private-value"
            }
        }))
        .expect("serialize auth");
        let (url, server) = mock_token_server(
            500,
            json!({"error": "backend_failure", "detail": "refresh-private-value"}),
        );
        let client = http_client().expect("http client");

        let error = refresh_auth_with(&client, &url, &auth).expect_err("transient error");
        let message = error.to_string();
        assert!(!message.contains("access-private-value"));
        assert!(!message.contains("refresh-private-value"));
        server.join().expect("join mock server");
    }

    #[test]
    fn authorization_url_and_callback_enforce_pkce_and_state() {
        let challenge = pkce_challenge("fixture-verifier");
        let url = build_authorize_url(
            "http://localhost:1455/auth/callback",
            &challenge,
            "fixture-state",
        )
        .expect("build authorize url");
        let params = url
            .query_pairs()
            .into_owned()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            params.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(
            params.get("state").map(String::as_str),
            Some("fixture-state")
        );
        assert!(matches!(
            parse_callback("/auth/callback?code=one&state=wrong", "fixture-state"),
            Callback::StateMismatch
        ));
        assert!(matches!(
            parse_callback("/auth/callback?code=one&state=fixture-state", "fixture-state"),
            Callback::Code(code) if code == "one"
        ));
    }

    #[test]
    fn callback_wait_stops_within_one_poll_after_cancellation() {
        let server = Server::http("127.0.0.1:0").expect("bind callback server");
        let cancelled = Arc::new(AtomicBool::new(false));
        let trigger = Arc::clone(&cancelled);
        let cancel_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            trigger.store(true, Ordering::SeqCst);
        });

        let started = Instant::now();
        let error = wait_for_authorization_code(
            &server,
            "fixture-state",
            Duration::from_secs(5),
            cancelled.as_ref(),
        )
        .expect_err("cancel callback wait");

        cancel_thread.join().expect("join cancellation thread");
        assert_eq!(error.to_string(), "官方登录已取消");
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
