use crate::auth::StoredAuth;
use crate::templates::RenderedTemplate;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api";

pub struct RunOptions {
    pub model: String,
    pub session_id: Option<String>,
}

pub fn run(auth: &StoredAuth, template: &RenderedTemplate, options: &RunOptions) -> Result<()> {
    let client = Client::builder().build()?;
    let url = resolve_url(DEFAULT_BASE_URL);
    let body = build_body(template, options);
    let headers = build_headers(auth, options.session_id.as_deref())?;

    let response = client.post(url).headers(headers).json(&body).send()?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format!("backend request failed: {status} {body}").into());
    }

    let mut printed = false;
    let mut completed_text: Option<String> = None;
    let mut reader = BufReader::new(response);
    let mut line = String::new();
    let mut data_lines: Vec<String> = Vec::new();

    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }

        let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
        if trimmed.is_empty() {
            if let Some(text) = process_event(&data_lines, &mut printed)? {
                completed_text = Some(text);
            }
            data_lines.clear();
            continue;
        }

        if let Some(data) = trimmed.strip_prefix("data:") {
            data_lines.push(data.trim().to_string());
        }
    }

    if let Some(text) = completed_text {
        if !printed && !text.is_empty() {
            print!("{text}");
            printed = true;
        }
    }

    if printed {
        println!();
    }

    Ok(())
}

fn process_event(data_lines: &[String], printed: &mut bool) -> Result<Option<String>> {
    if data_lines.is_empty() {
        return Ok(None);
    }

    let payload = data_lines.join("\n");
    if payload == "[DONE]" {
        return Ok(None);
    }

    let value: Value = serde_json::from_str(&payload)?;
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();

    match event_type {
        "response.output_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                print!("{delta}");
                std::io::stdout().flush()?;
                *printed = true;
            }
            Ok(None)
        }
        "error" => Err(format!("backend error: {payload}").into()),
        "response.failed" => Err(format!("backend failed: {payload}").into()),
        "response.completed" => Ok(extract_completed_text(&value)),
        _ => Ok(None),
    }
}

fn extract_completed_text(value: &Value) -> Option<String> {
    let output = value
        .get("response")
        .and_then(|response| response.get("output"))
        .and_then(Value::as_array)?;

    let mut text = String::new();
    for item in output {
        let contents = item
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for content in contents {
            if content.get("type").and_then(Value::as_str) == Some("output_text") {
                if let Some(part) = content.get("text").and_then(Value::as_str) {
                    text.push_str(part);
                }
            }
        }
    }

    Some(text)
}

fn build_body(template: &RenderedTemplate, options: &RunOptions) -> Value {
    let instructions = if template.system.trim().is_empty() {
        "You are a helpful assistant.".to_string()
    } else {
        template.system.clone()
    };

    json!({
        "model": options.model,
        "store": false,
        "stream": true,
        "instructions": instructions,
        "input": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": template.user
                    }
                ]
            }
        ],
        "text": { "verbosity": "low" },
        "include": ["reasoning.encrypted_content"],
        "prompt_cache_key": options.session_id,
        "tool_choice": "auto",
        "parallel_tool_calls": true
    })
}

fn build_headers(auth: &StoredAuth, session_id: Option<&str>) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", auth.access_token))?,
    );
    headers.insert(
        "chatgpt-account-id",
        HeaderValue::from_str(auth.account_id.as_str())?,
    );
    headers.insert(
        "OpenAI-Beta",
        HeaderValue::from_static("responses=experimental"),
    );
    headers.insert("originator", HeaderValue::from_static("pi"));
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("codex-oauth-cli/0.1.0"),
    );
    headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(session_id) = session_id {
        headers.insert("session_id", HeaderValue::from_str(session_id)?);
    }
    Ok(headers)
}

fn resolve_url(base_url: &str) -> String {
    let normalized = base_url.trim_end_matches('/');
    if normalized.ends_with("/codex/responses") {
        normalized.to_string()
    } else if normalized.ends_with("/codex") {
        format!("{normalized}/responses")
    } else {
        format!("{normalized}/codex/responses")
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_url;

    #[test]
    fn normalizes_base_url() {
        assert_eq!(
            resolve_url("https://chatgpt.com/backend-api"),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            resolve_url("https://chatgpt.com/backend-api/codex"),
            "https://chatgpt.com/backend-api/codex/responses"
        );
    }
}
