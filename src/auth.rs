use base64::Engine;
use rand::RngCore;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;

pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const JWT_CLAIM_PATH: &str = "https://api.openai.com/auth";
const OAUTH_SCOPE: &str = "openid profile email offline_access";
const SUCCESS_HTML: &str = "<!doctype html><html><body><p>Authentication successful. Return to your terminal.</p></body></html>";
const REFRESH_SKEW_MS: u64 = 60_000;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAuth {
    pub provider: String,
    pub access_token: String,
    pub refresh_token: String,
    pub account_id: String,
    pub expires_at_ms: u64,
    pub source: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

pub fn auth_file(config_dir: &Path) -> PathBuf {
    config_dir.join("auth.json")
}

pub fn load(config_dir: &Path) -> Result<StoredAuth> {
    let path = auth_file(config_dir);
    let raw = fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn save(config_dir: &Path, auth: &StoredAuth) -> Result<()> {
    fs::create_dir_all(config_dir)?;
    let path = auth_file(config_dir);
    let body = serde_json::to_string_pretty(auth)?;
    fs::write(path, format!("{body}\n"))?;
    Ok(())
}

pub fn remove(config_dir: &Path) -> Result<()> {
    let path = auth_file(config_dir);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn login(config_dir: &Path, manual_only: bool) -> Result<StoredAuth> {
    let client = oauth_client()?;
    let pkce = generate_pkce();
    let state = random_hex(16);
    let authorize_url = build_authorize_url(&pkce.challenge, &state)?;

    let listener = if manual_only {
        None
    } else {
        bind_callback_listener().ok()
    };

    println!("Open this URL in your browser:");
    println!("{authorize_url}");

    if !manual_only {
        let _ = open_in_browser(authorize_url.as_str());
    }

    let code = match listener {
        Some(listener) => {
            println!("Waiting for callback on {REDIRECT_URI}");
            wait_for_code(listener, &state, Duration::from_secs(180))?
                .or_else(|| prompt_for_code(&state).ok())
                .ok_or("missing authorization code")?
        }
        None => prompt_for_code(&state)?,
    };

    let token = exchange_code(&client, &code, &pkce.verifier)?;
    let access_token = token
        .access_token
        .ok_or("token exchange returned no access_token")?;
    let refresh_token = token
        .refresh_token
        .ok_or("token exchange returned no refresh_token")?;
    let expires_in = token
        .expires_in
        .ok_or("token exchange returned no expires_in")?;
    let account_id = extract_account_id(&access_token)?;
    let auth = StoredAuth {
        provider: "openai-codex".to_string(),
        access_token,
        refresh_token,
        account_id,
        expires_at_ms: now_ms().saturating_add(expires_in.saturating_mul(1000)),
        source: "oauth-browser".to_string(),
    };
    save(config_dir, &auth)?;
    Ok(auth)
}

pub fn ensure_fresh(config_dir: &Path, auth: &StoredAuth) -> Result<StoredAuth> {
    if auth.expires_at_ms > now_ms().saturating_add(REFRESH_SKEW_MS) {
        return Ok(auth.clone());
    }
    refresh(config_dir, auth)
}

pub fn refresh(config_dir: &Path, auth: &StoredAuth) -> Result<StoredAuth> {
    let client = oauth_client()?;
    let token = refresh_token(&client, &auth.refresh_token)?;
    let access_token = token
        .access_token
        .ok_or("refresh returned no access_token")?;
    let refresh_token = token
        .refresh_token
        .unwrap_or_else(|| auth.refresh_token.clone());
    let expires_in = token.expires_in.ok_or("refresh returned no expires_in")?;
    let refreshed = StoredAuth {
        provider: auth.provider.clone(),
        account_id: extract_account_id(&access_token)?,
        access_token,
        refresh_token,
        expires_at_ms: now_ms().saturating_add(expires_in.saturating_mul(1000)),
        source: auth.source.clone(),
    };
    save(config_dir, &refreshed)?;
    Ok(refreshed)
}

#[cfg(test)]
pub fn decode_jwt_exp_ms(token: &str) -> Option<u64> {
    let claims = decode_jwt_claims(token).ok()?;
    claims.get("exp")?.as_u64().map(|v| v.saturating_mul(1000))
}

pub fn extract_account_id(token: &str) -> Result<String> {
    let claims = decode_jwt_claims(token)?;
    let account_id = claims
        .get(JWT_CLAIM_PATH)
        .and_then(Value::as_object)
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("failed to extract account_id from token")?;
    Ok(account_id.to_string())
}

fn prompt_for_code(expected_state: &str) -> Result<String> {
    println!("Paste the redirect URL or authorization code:");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    parse_authorization_input(input.trim(), expected_state)
}

fn parse_authorization_input(input: &str, expected_state: &str) -> Result<String> {
    if input.is_empty() {
        return Err("empty authorization input".into());
    }

    if let Ok(url) = Url::parse(input) {
        let state = url
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.to_string());
        if let Some(state) = state {
            ensure_state(&state, expected_state)?;
        }
        let code = url
            .query_pairs()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.to_string())
            .ok_or("redirect URL missing code")?;
        return Ok(code);
    }

    if let Some((code, state)) = input.split_once('#') {
        ensure_state(state, expected_state)?;
        return Ok(code.to_string());
    }

    if input.contains("code=") {
        let params = url::form_urlencoded::parse(input.as_bytes())
            .into_owned()
            .collect::<Vec<_>>();
        let code = params
            .iter()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.to_string())
            .ok_or("authorization input missing code")?;
        if let Some(state) = params
            .iter()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.to_string())
        {
            ensure_state(&state, expected_state)?;
        }
        return Ok(code);
    }

    Ok(input.to_string())
}

fn ensure_state(actual: &str, expected: &str) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err("oauth state mismatch".into())
    }
}

fn bind_callback_listener() -> Result<TcpListener> {
    Ok(TcpListener::bind("127.0.0.1:1455")?)
}

fn wait_for_code(
    listener: TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<Option<String>> {
    listener.set_nonblocking(true)?;
    let started = Instant::now();

    while started.elapsed() < timeout {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut buffer = [0_u8; 8192];
                let size = stream.read(&mut buffer)?;
                let request = String::from_utf8_lossy(&buffer[..size]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let url = Url::parse(&format!("http://localhost{path}"))?;
                let state = url
                    .query_pairs()
                    .find(|(k, _)| k == "state")
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_default();
                let code = url
                    .query_pairs()
                    .find(|(k, _)| k == "code")
                    .map(|(_, v)| v.to_string());

                let (status, body) = if state != expected_state {
                    ("400 Bad Request", "State mismatch")
                } else if code.is_none() {
                    ("400 Bad Request", "Missing authorization code")
                } else {
                    ("200 OK", SUCCESS_HTML)
                };

                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes())?;
                stream.flush()?;

                if state == expected_state {
                    if let Some(code) = code {
                        return Ok(Some(code));
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(err) => return Err(err.into()),
        }
    }

    Ok(None)
}

fn build_authorize_url(challenge: &str, state: &str) -> Result<Url> {
    let mut url = Url::parse(AUTHORIZE_URL)?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("response_type", "code");
        pairs.append_pair("client_id", CLIENT_ID);
        pairs.append_pair("redirect_uri", REDIRECT_URI);
        pairs.append_pair("scope", OAUTH_SCOPE);
        pairs.append_pair("code_challenge", challenge);
        pairs.append_pair("code_challenge_method", "S256");
        pairs.append_pair("state", state);
        pairs.append_pair("id_token_add_organizations", "true");
        pairs.append_pair("codex_cli_simplified_flow", "true");
        pairs.append_pair("originator", "pi");
    }
    Ok(url)
}

fn oauth_client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent("codex-oauth-cli/0.1.0")
        .build()?)
}

fn open_in_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).status()?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).status()?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", url])
            .status()?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Ok(())
}

fn exchange_code(client: &Client, code: &str, verifier: &str) -> Result<TokenResponse> {
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", REDIRECT_URI),
        ])
        .send()?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format!("oauth token exchange failed: {status} {body}").into());
    }

    Ok(response.json()?)
}

fn refresh_token(client: &Client, refresh_token: &str) -> Result<TokenResponse> {
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", refresh_token),
        ])
        .send()?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format!("oauth refresh failed: {status} {body}").into());
    }

    Ok(response.json()?)
}

struct PkcePair {
    verifier: String,
    challenge: String,
}

fn generate_pkce() -> PkcePair {
    let mut random = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut random);
    let verifier = base64_url_encode(&random);
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = base64_url_encode(&hasher.finalize());
    PkcePair {
        verifier,
        challenge,
    }
}

fn decode_jwt_claims(token: &str) -> Result<Value> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or("jwt missing payload segment")?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload)?;
    Ok(serde_json::from_slice(&decoded)?)
}

fn base64_url_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random_hex(byte_len: usize) -> String {
    let mut bytes = vec![0_u8; byte_len];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::{decode_jwt_exp_ms, parse_authorization_input};

    #[test]
    fn parses_redirect_value() {
        let input = "http://localhost:1455/auth/callback?code=abc&state=xyz";
        let code = parse_authorization_input(input, "xyz").unwrap();
        assert_eq!(code, "abc");
    }

    #[test]
    fn rejects_state_mismatch() {
        let input = "http://localhost:1455/auth/callback?code=abc&state=nope";
        assert!(parse_authorization_input(input, "xyz").is_err());
    }

    #[test]
    fn decodes_exp_field() {
        let token = "aaa.eyJleHAiOjE3MDAwMDAwMDB9.bbb";
        assert_eq!(decode_jwt_exp_ms(token), Some(1_700_000_000_000));
    }
}
