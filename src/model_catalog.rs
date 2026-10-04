use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::time::Duration;

pub async fn provider_models(provider: &Value) -> Result<Value> {
    let base = provider["base_url"]
        .as_str()
        .context("模型 provider 缺少 base_url")?;
    let mut url = reqwest::Url::parse(base).map_err(|_| anyhow!("模型 provider base_url 无效"))?;
    url.set_path(&format!("{}/models", url.path().trim_end_matches('/')));
    url.set_fragment(None);
    if let Some(params) = provider["query_params"].as_object() {
        for (key, value) in params {
            url.query_pairs_mut().append_pair(
                key,
                value
                    .as_str()
                    .context("provider query_params 必须为字符串")?,
            );
        }
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut request = client.get(url);
    let token = if let Some(key) = provider["env_key"].as_str() {
        Some(
            std::env::var(key)
                .ok()
                .filter(|s| !s.trim().is_empty())
                .context("模型 provider 的鉴权环境变量未设置")?,
        )
    } else {
        provider["experimental_bearer_token"]
            .as_str()
            .map(str::to_owned)
    };
    if let Some(token) = token {
        request = request.bearer_auth(token);
    } else if provider["requires_openai_auth"].as_bool() == Some(true) {
        bail!(
            "该自定义 provider 需要 OpenAI 登录鉴权；动态模型查询需要配置 env_key 或 bearer token"
        );
    }
    for field in ["http_headers", "env_http_headers"] {
        if let Some(headers) = provider[field].as_object() {
            for (name, value) in headers {
                let value = value
                    .as_str()
                    .context("模型 provider header 必须为字符串")?;
                let value = if field == "env_http_headers" {
                    std::env::var(value)
                        .map_err(|_| anyhow!("模型 provider header 环境变量未设置"))?
                } else {
                    value.to_owned()
                };
                request = request.header(name, value);
            }
        }
    }
    let response = request
        .send()
        .await
        .map_err(|_| anyhow!("模型 provider /models 请求失败（连接、超时或请求配置错误）"))?;
    if !response.status().is_success() {
        bail!(
            "模型 provider /models 返回 HTTP {}",
            response.status().as_u16()
        );
    }
    let body: Value = response
        .json()
        .await
        .map_err(|_| anyhow!("模型 provider /models 返回无效 JSON"))?;
    let rows = body["data"]
        .as_array()
        .context("模型 provider /models 缺少 data 数组")?;
    let mut seen = std::collections::HashSet::new();
    let mut data = Vec::new();
    for row in rows {
        let id = row["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("模型 provider /models 返回无效模型 ID")?;
        if !seen.insert(id) {
            continue;
        }
        data.push(json!({
            "id":id,"model":id,
            "displayName":row["displayName"].as_str().unwrap_or(id),
            "description":row["description"].as_str().unwrap_or("Provider /models"),
            "defaultReasoningEffort":row["defaultReasoningEffort"],
            "supportedReasoningEfforts":row.get("supportedReasoningEfforts").cloned().unwrap_or(json!([]))
        }));
    }
    Ok(json!({"data":data,"nextCursor":null}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn query(status: u16, body: &str) -> (Result<Value>, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = body.to_owned();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 1024];
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&chunk[..n]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        let result = provider_models(&json!({
            "base_url":format!("http://{address}/v1/"),
            "experimental_bearer_token":"test-secret",
            "http_headers":{"x-provider":"custom"},
            "query_params":{"version":"test"}
        }))
        .await;
        (result, server.await.unwrap())
    }

    #[tokio::test]
    async fn queries_provider_with_auth_and_preserves_only_returned_models() {
        let (result, request) = query(
            200,
            r#"{"data":[{"id":"qwen3.8-27b"},{"id":"qwen3.8-27b"},{"id":"other"}]}"#,
        )
        .await;
        let result = result.unwrap();
        assert_eq!(result["data"].as_array().unwrap().len(), 2);
        assert_eq!(result["data"][0]["id"], "qwen3.8-27b");
        assert!(result["data"][0]["defaultReasoningEffort"].is_null());
        assert_eq!(result["data"][0]["supportedReasoningEfforts"], json!([]));
        assert!(request.starts_with("GET /v1/models?version=test HTTP/1.1"));
        assert!(request.contains("authorization: Bearer test-secret"));
        assert!(request.contains("x-provider: custom"));
    }

    #[tokio::test]
    async fn errors_are_explicit_and_do_not_leak_response_or_credentials() {
        for (status, body, expected) in [
            (401, "sensitive-response", "HTTP 401"),
            (302, "sensitive-response", "HTTP 302"),
            (200, "sensitive-response", "JSON"),
            (200, "{}", "data"),
            (200, r#"{"data":[{}]}"#, "ID"),
        ] {
            let error = query(status, body).await.0.unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("sensitive-response"));
            assert!(!error.contains("test-secret"));
        }
    }
}
