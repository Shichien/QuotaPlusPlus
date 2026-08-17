use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde::Serialize;
use std::error::Error;
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    pub protocol: String,
    pub inference_endpoint: String,
    pub anthropic_auth: bool,
    pub routing_required: bool,
    pub message: String,
}

pub fn detect(api_url: &str, api_key: &str) -> Result<Detection, Box<dyn Error>> {
    let client = Client::builder()
        .timeout(PROBE_TIMEOUT)
        .redirect(Policy::none())
        .user_agent(concat!("CSwitch/", env!("CARGO_PKG_VERSION")))
        .build()?;

    let responses = probe_protocol(&client, api_url, api_key, "responses", false)?;
    if let Some(endpoint) = responses {
        return Ok(Detection {
            protocol: "openai_responses".to_string(),
            inference_endpoint: endpoint,
            anthropic_auth: false,
            routing_required: false,
            message: "上游提供 Responses，使用直连模式".to_string(),
        });
    }

    if let Some(endpoint) = probe_protocol(&client, api_url, api_key, "chat", false)? {
        return Ok(Detection {
            protocol: "openai_chat".to_string(),
            inference_endpoint: endpoint,
            anthropic_auth: false,
            routing_required: true,
            message: "上游不提供 Responses，仅支持 OpenAI Chat Completions。是否启动本地路由进行协议转换？".to_string(),
        });
    }

    if let Some(endpoint) = probe_protocol(&client, api_url, api_key, "anthropic", true)? {
        return Ok(Detection {
            protocol: "anthropic_messages".to_string(),
            inference_endpoint: endpoint,
            anthropic_auth: true,
            routing_required: true,
            message:
                "上游不提供 Responses，仅支持 Anthropic Messages。是否启动本地路由进行协议转换？"
                    .to_string(),
        });
    }

    Err("没有检测到可用的 Responses、Chat Completions 或 Anthropic Messages 接口".into())
}

pub fn protocol_label(protocol: &str) -> &'static str {
    match protocol {
        "openai_responses" => "Responses",
        "openai_chat" => "OpenAI Chat Completions",
        "anthropic_messages" => "Anthropic Messages",
        _ => "未知协议",
    }
}

pub fn base_url_for_endpoint(endpoint: &str, protocol: &str) -> Result<String, Box<dyn Error>> {
    let suffix = match protocol {
        "openai_responses" => "/responses",
        "openai_chat" => "/chat/completions",
        "anthropic_messages" => "/messages",
        _ => return Err(format!("不支持的上游协议：{protocol}").into()),
    };
    endpoint
        .strip_suffix(suffix)
        .map(|base| base.trim_end_matches('/').to_string())
        .filter(|base| !base.is_empty())
        .ok_or_else(|| format!("推理接口地址与协议不匹配：{endpoint}").into())
}

fn probe_protocol(
    client: &Client,
    api_url: &str,
    api_key: &str,
    protocol: &str,
    anthropic_auth: bool,
) -> Result<Option<String>, Box<dyn Error>> {
    for endpoint in endpoint_candidates(api_url, protocol) {
        let request = client.post(&endpoint).json(&serde_json::json!({}));
        let response = if anthropic_auth {
            request
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .send()
        } else {
            request.bearer_auth(api_key).send()
        };
        let response = response.map_err(|error| format!("探测上游接口失败 {endpoint}：{error}"))?;
        let status = response.status();
        if matches!(
            status,
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            return Err(format!("API Key 未通过上游验证 {endpoint}，返回 {status}").into());
        }
        if status.is_redirection() {
            return Err(format!("上游接口发生重定向 {endpoint}，返回 {status}").into());
        }
        if status != reqwest::StatusCode::NOT_FOUND
            && status != reqwest::StatusCode::METHOD_NOT_ALLOWED
            && status != reqwest::StatusCode::GONE
        {
            return Ok(Some(endpoint));
        }
    }
    Ok(None)
}

fn endpoint_candidates(api_url: &str, protocol: &str) -> Vec<String> {
    let base = api_url.trim_end_matches('/');
    let suffix = match protocol {
        "responses" => "responses",
        "chat" => "chat/completions",
        "anthropic" => "messages",
        _ => return Vec::new(),
    };
    let mut candidates = Vec::new();
    if base.ends_with("/v1") {
        candidates.push(format!("{base}/{suffix}"));
    } else {
        candidates.push(format!("{base}/{suffix}"));
        candidates.push(format!("{base}/v1/{suffix}"));
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use tiny_http::{Response, Server, StatusCode};

    #[test]
    fn candidates_cover_root_and_v1_without_duplicate_v1() {
        assert_eq!(
            endpoint_candidates("https://api.example.com", "responses"),
            [
                "https://api.example.com/responses",
                "https://api.example.com/v1/responses"
            ]
        );
        assert_eq!(
            endpoint_candidates("https://api.example.com/v1", "chat"),
            ["https://api.example.com/v1/chat/completions"]
        );
    }

    #[test]
    fn detects_responses_without_trying_other_protocols() {
        let server = Server::http("127.0.0.1:0").expect("server");
        let address = server.server_addr();
        let handle = thread::spawn(move || {
            let request = server.recv().expect("request");
            assert!(request.url().ends_with("/responses"));
            request
                .respond(Response::empty(StatusCode(400)))
                .expect("respond responses probe");
        });

        let result = detect(&format!("http://{address}"), "key").expect("detect");
        assert_eq!(result.protocol, "openai_responses");
        assert!(!result.routing_required);
        handle.join().expect("join");
    }

    #[test]
    fn detects_chat_when_responses_is_missing() {
        let server = Server::http("127.0.0.1:0").expect("server");
        let address = server.server_addr();
        let handle = thread::spawn(move || {
            for _ in 0..3 {
                let request = server.recv().expect("request");
                let status = if request.url().ends_with("/responses") {
                    StatusCode(404)
                } else {
                    StatusCode(400)
                };
                request.respond(Response::empty(status)).expect("response");
            }
        });
        let result = detect(&format!("http://{address}"), "key").expect("detect");
        assert_eq!(result.protocol, "openai_chat");
        assert!(result.routing_required);
        handle.join().expect("join");
    }

    #[test]
    fn detects_anthropic_after_openai_protocols_are_missing() {
        let server = Server::http("127.0.0.1:0").expect("server");
        let address = server.server_addr();
        let handle = thread::spawn(move || {
            for _ in 0..5 {
                let request = server.recv().expect("request");
                if request.url().ends_with("/messages") {
                    assert!(
                        request
                            .headers()
                            .iter()
                            .any(|header| header.field.equiv("x-api-key")
                                && header.value.as_str() == "key")
                    );
                    assert!(
                        request
                            .headers()
                            .iter()
                            .any(|header| header.field.equiv("anthropic-version"))
                    );
                    request
                        .respond(Response::empty(StatusCode(400)))
                        .expect("respond messages probe");
                } else {
                    request
                        .respond(Response::empty(StatusCode(404)))
                        .expect("respond missing OpenAI endpoint");
                }
            }
        });

        let result = detect(&format!("http://{address}"), "key").expect("detect");
        assert_eq!(result.protocol, "anthropic_messages");
        assert!(result.routing_required);
        assert!(result.anthropic_auth);
        handle.join().expect("join");
    }

    #[test]
    fn derives_the_codex_base_url_from_the_detected_endpoint() {
        assert_eq!(
            base_url_for_endpoint("https://api.example.com/v1/responses", "openai_responses")
                .unwrap(),
            "https://api.example.com/v1"
        );
        assert_eq!(
            base_url_for_endpoint("https://api.example.com/chat/completions", "openai_chat")
                .unwrap(),
            "https://api.example.com"
        );
    }
}
