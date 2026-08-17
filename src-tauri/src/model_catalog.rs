use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::error::Error;
use std::time::Duration;

const MODELS_TIMEOUT: Duration = Duration::from_secs(20);
#[derive(Debug)]
pub struct ModelCatalog {
    pub bytes: Vec<u8>,
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<RemoteModel>,
}

#[derive(Deserialize)]
struct RemoteModel {
    id: String,
}

#[cfg(test)]
pub fn fetch(api_url: &str, api_key: &str) -> Result<ModelCatalog, Box<dyn Error>> {
    fetch_with_auth(api_url, api_key, false)
}

pub fn fetch_with_auth(
    api_url: &str,
    api_key: &str,
    anthropic_auth: bool,
) -> Result<ModelCatalog, Box<dyn Error>> {
    let client = Client::builder()
        .timeout(MODELS_TIMEOUT)
        .redirect(Policy::none())
        .user_agent(concat!("CSwitch/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let endpoint = models_endpoint(api_url);
    let request = client.get(&endpoint);
    let response = if anthropic_auth {
        request
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
    } else {
        request.bearer_auth(api_key).send()
    }
    .map_err(|error| format!("连接模型列表失败 {endpoint}：{error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("模型列表请求失败 {endpoint}，返回 {status}").into());
    }
    let payload: ModelsResponse = response
        .json()
        .map_err(|error| format!("模型列表不是标准的 data[].id 格式：{error}"))?;
    build(payload.data.into_iter().map(|model| model.id))
}

pub fn build<I>(model_ids: I) -> Result<ModelCatalog, Box<dyn Error>>
where
    I: IntoIterator<Item = String>,
{
    let mut seen = HashSet::new();
    let model_ids = model_ids
        .into_iter()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .filter(|model| seen.insert(model.clone()))
        .collect::<Vec<_>>();
    if model_ids.is_empty() {
        return Err("模型列表中没有可用的模型 ID".into());
    }

    let models = model_ids
        .iter()
        .map(|model| model_info(model))
        .collect::<Vec<_>>();
    Ok(ModelCatalog {
        bytes: serde_json::to_vec_pretty(&json!({ "models": models }))?,
    })
}

fn models_endpoint(api_url: &str) -> String {
    let base = api_url.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    }
}

fn model_info(model: &str) -> Value {
    json!({
        "slug": model,
        "display_name": model
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use tiny_http::{Header, Response, Server};

    #[test]
    fn root_and_v1_urls_resolve_without_duplicate_segments() {
        assert_eq!(
            models_endpoint("https://api.example.com"),
            "https://api.example.com/v1/models"
        );
        assert_eq!(
            models_endpoint("https://api.example.com/v1"),
            "https://api.example.com/v1/models"
        );
    }

    #[test]
    fn catalog_preserves_order_and_removes_duplicate_ids() {
        let catalog = build([
            "model-b".to_string(),
            " model-a ".to_string(),
            "model-b".to_string(),
        ])
        .expect("build catalog");
        let value: Value = serde_json::from_slice(&catalog.bytes).expect("parse catalog");
        let ids = value["models"]
            .as_array()
            .expect("models array")
            .iter()
            .map(|model| model["slug"].as_str().expect("slug"))
            .collect::<Vec<_>>();
        assert_eq!(ids, ["model-b", "model-a"]);
        assert_eq!(
            value["models"],
            json!([
                {
                    "slug": "model-b",
                    "display_name": "model-b"
                },
                {
                    "slug": "model-a",
                    "display_name": "model-a"
                }
            ])
        );
    }

    #[test]
    fn fetches_standard_models_with_bearer_auth() {
        let server = Server::http("127.0.0.1:0").expect("bind server");
        let address = server.server_addr();
        let handle = thread::spawn(move || {
            let mut request = server.recv().expect("receive request");
            assert_eq!(request.method().as_str(), "GET");
            assert_eq!(request.url(), "/v1/models");
            let authorization = request
                .headers()
                .iter()
                .find(|header| header.field.equiv("Authorization"))
                .expect("authorization");
            assert_eq!(authorization.value.as_str(), "Bearer fixture-key");
            let mut body = String::new();
            request
                .as_reader()
                .read_to_string(&mut body)
                .expect("read request");
            assert!(body.is_empty());
            request
                .respond(
                    Response::from_string(r#"{"data":[{"id":"model-a"},{"id":"model-b"}]}"#)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json")
                                .expect("content type"),
                        ),
                )
                .expect("respond");
        });

        let catalog = fetch(&format!("http://{address}"), "fixture-key").expect("fetch catalog");
        let value: Value = serde_json::from_slice(&catalog.bytes).expect("parse catalog");
        let ids = value["models"]
            .as_array()
            .expect("models array")
            .iter()
            .map(|model| model["slug"].as_str().expect("slug"))
            .collect::<Vec<_>>();
        assert_eq!(ids, ["model-a", "model-b"]);
        handle.join().expect("join server");
    }
}
