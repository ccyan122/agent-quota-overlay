//! Read-only diagnostic. It prints normalized quota only: no credentials, account IDs, or raw responses.
use llm_quota_overlay::{auth, providers::{fetch_all, fetch_selected}, quota::ProviderId};

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.iter().any(|argument| argument == "--credential-presence-only") {
        println!("{}", serde_json::json!({
            "chatgptWork": auth::codex::credentials_present(),
            "claudeCode": auth::claude::credentials_present(),
            "antigravity": auth::antigravity::credentials_present(),
        }));
        return;
    }
    let results = match provider_from_args(&arguments) {
        Ok(Some(provider)) => fetch_selected(&[provider]),
        Ok(None) => fetch_all(),
        Err(message) => { eprintln!("{message}"); std::process::exit(2); }
    };
    let output: Vec<serde_json::Value> = results.into_iter().map(|(provider, result)| match result {
        Ok(quota) => serde_json::json!({"provider": provider, "status": "ok", "quota": quota}),
        Err(error) => serde_json::json!({"provider": provider, "status": "unavailable", "reason": error.to_string()}),
    }).collect();
    println!("{}", serde_json::to_string_pretty(&output).expect("JSON serialization"));
}

fn provider_from_args(arguments: &[String]) -> Result<Option<ProviderId>, &'static str> {
    let Some(index) = arguments.iter().position(|argument| argument == "--provider") else { return Ok(None); };
    let Some(value) = arguments.get(index + 1) else { return Err("--provider requires chatgpt-work, claude-code, or antigravity"); };
    match value.as_str() {
        "chatgpt-work" => Ok(Some(ProviderId::ChatgptWork)),
        "claude-code" => Ok(Some(ProviderId::ClaudeCode)),
        "antigravity" => Ok(Some(ProviderId::Antigravity)),
        _ => Err("--provider requires chatgpt-work, claude-code, or antigravity"),
    }
}
