use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde_json::json;
use std::error::Error;
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

pub fn validate_proxy(api_url: &str, api_key: &str) -> Result<(), Box<dyn Error>> {
    let client = Client::builder()
        .timeout(PROBE_TIMEOUT)
        .redirect(Policy::none())
        .user_agent(concat!("QuotaPlusPlus/", env!("CARGO_PKG_VERSION")))
        .build()?;
    validate_proxy_with(&client, api_url, api_key)
}

fn validate_proxy_with(
    client: &Client,
    api_url: &str,
    api_key: &str,
) -> Result<(), Box<dyn Error>> {
    let endpoint = format!("{}/responses", api_url.trim_end_matches('/'));
    let response = client
        .post(endpoint)
        .bearer_auth(api_key)
        .json(&json!({}))
        .send()
        .map_err(|error| format!("连接 Responses API 失败：{error}"))?;
    let status = response.status();

    if status.is_success()
        || matches!(
            status,
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY
        )
    {
        return Ok(());
    }

    let message = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            format!("API Key 验证失败，Responses API 返回 {status}")
        }
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => {
            format!("API URL 没有可用的 Responses 端点，返回 {status}")
        }
        StatusCode::TOO_MANY_REQUESTS => {
            format!("Responses API 当前没有可用额度，返回 {status}")
        }
        _ if status.is_server_error() => {
            format!("Responses API 服务异常，返回 {status}")
        }
        _ => format!("Responses API 预检失败，返回 {status}"),
    };
    Err(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use tiny_http::{Response, Server, StatusCode as TinyStatusCode};

    fn mock_server(status: u16) -> (String, thread::JoinHandle<()>) {
        let server = Server::http("127.0.0.1:0").expect("bind mock server");
        let address = server.server_addr();
        let handle = thread::spawn(move || {
            let mut request = server.recv().expect("receive probe");
            assert_eq!(request.method().as_str(), "POST");
            assert_eq!(request.url(), "/v1/responses");
            let authorization = request
                .headers()
                .iter()
                .find(|header| header.field.equiv("Authorization"))
                .expect("authorization header");
            assert_eq!(authorization.value.as_str(), "Bearer fixture-key");
            let mut body = String::new();
            request
                .as_reader()
                .read_to_string(&mut body)
                .expect("read request body");
            assert_eq!(body, "{}");
            request
                .respond(Response::empty(TinyStatusCode(status)))
                .expect("respond to probe");
        });
        (format!("http://{address}/v1"), handle)
    }

    #[test]
    fn accepts_schema_error_from_existing_responses_endpoint() {
        let (url, server) = mock_server(400);
        validate_proxy(&url, "fixture-key").expect("accept endpoint");
        server.join().expect("join server");
    }

    #[test]
    fn rejects_invalid_proxy_credentials() {
        let (url, server) = mock_server(401);
        let error = validate_proxy(&url, "fixture-key").expect_err("reject credentials");
        assert!(error.to_string().contains("API Key 验证失败"));
        server.join().expect("join server");
    }
}
