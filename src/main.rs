mod auth;
mod client;

use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let config_dir = config_dir();
    let args = std::env::args().skip(1).collect::<Vec<_>>();

    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        print_usage(&config_dir);
        return Ok(());
    }

    match args[0].as_str() {
        "login" => {
            let manual_only = args.iter().any(|arg| arg == "--manual");
            let auth = auth::login(&config_dir, manual_only)?;
            println!("logged_in=true");
            println!("source={}", auth.source);
            println!("account_id={}", auth.account_id);
        }
        "status" => print_status(&config_dir)?,
        "logout" => {
            auth::remove(&config_dir)?;
            println!("logged_out=true");
        }
        "run" => handle_run(&config_dir, &args[1..])?,
        _ => handle_run(&config_dir, &args)?,
    }

    Ok(())
}

fn handle_run(config_dir: &PathBuf, args: &[String]) -> Result<()> {
    let mut model = "gpt-5.4".to_string();
    let mut session_id: Option<String> = None;
    let mut prompt_parts = Vec::new();

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--model" => {
                index += 1;
                model = args.get(index).ok_or("missing value for --model")?.clone();
            }
            "--session" => {
                index += 1;
                session_id = Some(
                    args.get(index)
                        .ok_or("missing value for --session")?
                        .clone(),
                );
            }
            value if value.starts_with('-') => return Err(format!("unknown flag: {value}").into()),
            value => prompt_parts.push(value.to_string()),
        }
        index += 1;
    }

    let prompt = if prompt_parts.is_empty() {
        return Err("missing prompt".into());
    } else {
        prompt_parts.join(" ")
    };

    let auth = auth::ensure_fresh(config_dir, &auth::load(config_dir)?)?;
    let options = client::RunOptions { model, session_id };
    client::run(&auth, &prompt, &options)?;
    Ok(())
}

fn print_status(config_dir: &PathBuf) -> Result<()> {
    let auth_path = auth::auth_file(config_dir);
    println!("config_dir={}", config_dir.display());
    println!("auth_file={}", auth_path.display());

    match auth::load(config_dir) {
        Ok(auth) => {
            println!("logged_in=true");
            println!("provider={}", auth.provider);
            println!("source={}", auth.source);
            println!("account_id={}", auth.account_id);
            println!("expires_at_utc={}", format_timestamp(auth.expires_at_ms));
            let remaining_ms = auth.expires_at_ms.saturating_sub(now_ms());
            println!("expires_in_seconds={}", remaining_ms / 1000);
        }
        Err(_) => {
            println!("logged_in=false");
        }
    }

    Ok(())
}

fn print_usage(config_dir: &PathBuf) {
    println!(
        "\
codex-oauth-cli

Standalone Rust CLI for ChatGPT/Codex OAuth and direct backend-api calls.

Usage:
  codex-oauth-cli login [--manual]
  codex-oauth-cli status
  codex-oauth-cli logout
  codex-oauth-cli run [--model MODEL] [--session ID] <prompt>
  codex-oauth-cli [--model MODEL] [--session ID] <prompt>

Notes:
  config_dir={}
  browser_oauth_callback=http://localhost:1455/auth/callback
",
        config_dir.display()
    );
}

fn config_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join("codex-oauth-cli")
}

fn format_timestamp(timestamp_ms: u64) -> String {
    let system_time = UNIX_EPOCH + Duration::from_millis(timestamp_ms);
    let datetime: chrono::DateTime<chrono::Utc> = system_time.into();
    datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64
}
