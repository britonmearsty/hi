use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::{self, Write},
    path::PathBuf,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub provider: String,
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub approval_mode: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: "openai".into(),
            api_key: String::new(),
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-4o-mini".into(),
            approval_mode: "always".into(),
        }
    }
}

pub fn config_path() -> PathBuf {
    if let Ok(path) = env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(path).join("hi/config.env");
    }
    PathBuf::from(env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config/hi/config.env")
}
pub fn data_path() -> PathBuf {
    if let Ok(path) = env::var("XDG_DATA_HOME") {
        return PathBuf::from(path).join("hi/sessions.db");
    }
    PathBuf::from(env::var("HOME").unwrap_or_else(|_| ".".into()))
        .join(".local/share/hi/sessions.db")
}

pub fn load() -> Result<Config> {
    let mut config = Config::default();
    if let Ok(contents) = fs::read_to_string(config_path()) {
        for line in contents.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "provider" => config.provider = value.trim().into(),
                "api_key" => config.api_key = value.trim().into(),
                "base_url" => config.base_url = value.trim().into(),
                "model" => config.model = value.trim().into(),
                "approval_mode" => config.approval_mode = value.trim().into(),
                _ => {}
            }
        }
    }
    if let Ok(value) = env::var("HI_API_KEY") {
        config.api_key = value;
    } else if config.api_key.is_empty() {
        if let Ok(entry) = keyring::Entry::new("hi", "api-key") {
            if let Ok(value) = entry.get_password() {
                config.api_key = value;
            }
        }
    }
    if let Ok(value) = env::var("HI_BASE_URL") {
        config.base_url = value;
    }
    if let Ok(value) = env::var("HI_MODEL") {
        config.model = value;
    }
    if let Ok(value) = env::var("HI_APPROVAL_MODE") {
        config.approval_mode = value;
    }
    Ok(config)
}

pub fn setup() -> Result<()> {
    let old = load()?;
    println!("\nhi setup\n=======");
    println!("Choose the provider for this installation:");
    println!("  1) OpenAI");
    println!("  2) OpenAI-compatible endpoint");
    println!("  3) Local endpoint");
    let provider_choice = prompt("Provider [1]", "1")?;
    let (provider, default_url, key_required) = match provider_choice.trim() {
        "2" => ("openai-compatible", old.base_url.clone(), false),
        "3" => ("local", "http://localhost:11434/v1".into(), false),
        _ => ("openai", "https://api.openai.com/v1".into(), true),
    };
    println!("\nStep 2 of 3: API credentials");
    let api_key = if key_required || !old.api_key.is_empty() {
        prompt_secret(
            if old.api_key.is_empty() {
                "API key"
            } else {
                "API key (press Enter to keep existing)"
            },
            &old.api_key,
        )?
    } else {
        prompt_secret("API key (optional for local endpoints)", "")?
    };
    let base_url = if provider == "openai" {
        default_url
    } else {
        prompt("Base URL", &default_url)?
    };
    println!("\nStep 3 of 3: model");
    println!("  1) gpt-4o-mini (recommended default)");
    println!("  2) gpt-4o");
    println!("  3) Enter a custom model ID");
    let model_choice = prompt("Model [1]", "1")?;
    let model = match model_choice.trim() {
        "2" => "gpt-4o".into(),
        "3" => prompt("Model ID", &old.model)?,
        _ => "gpt-4o-mini".into(),
    };
    println!("\nCommand approval");
    println!("  1) Always ask (recommended)");
    println!("  2) Auto-approve safe commands");
    println!("  3) Auto-approve all commands (unsafe)");
    let approval_mode = match prompt("Approval mode [1]", "1")?.trim() {
        "2" => "safe-only",
        "3" => "never",
        _ => "always",
    };
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let keyring_saved = if !api_key.is_empty() {
        keyring::Entry::new("hi", "api-key")
            .and_then(|entry| entry.set_password(&api_key).map(|_| ()))
            .is_ok()
    } else {
        false
    };
    let stored_key = &api_key;
    fs::write(
        &path,
        format!("provider={provider}\napi_key={stored_key}\nbase_url={base_url}\nmodel={model}\napproval_mode={approval_mode}\n"),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    let storage = if keyring_saved {
        "OS credential store and protected config fallback"
    } else {
        "protected 0600 config file"
    };
    println!("\nSetup complete. API key stored in {storage}.");
    println!("Saved configuration to {}", path.display());
    Ok(())
}
fn prompt(label: &str, default: &str) -> Result<String> {
    print!("{label} [{}]: ", default);
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim();
    Ok(if value.is_empty() {
        default.into()
    } else {
        value.into()
    })
}
fn prompt_secret(label: &str, existing: &str) -> Result<String> {
    let value = dialoguer::Password::new()
        .with_prompt(label)
        .allow_empty_password(true)
        .interact()?;
    Ok(if value.trim().is_empty() {
        existing.into()
    } else {
        value.trim().into()
    })
}
pub fn ensure_configured() -> Result<()> {
    let config = load()?;
    if !config_path().exists() || (config.api_key.is_empty() && config.provider == "openai") {
        println!("Welcome to hi. Let's configure your AI provider first.");
        setup()?;
        let configured = load()?;
        if configured.api_key.is_empty() && configured.provider != "local" {
            anyhow::bail!("API key was not saved; run `hi config` and try again");
        }
    }
    Ok(())
}
pub async fn doctor() -> Result<()> {
    let config = load()?;
    println!(
        "config: {}\nprovider: {}\napi key: {}\nbase URL: {}\nmodel: {}\napproval mode: {}",
        config_path().display(),
        config.provider,
        if config.api_key.is_empty() {
            "missing"
        } else {
            "configured"
        },
        config.base_url,
        config.model,
        config.approval_mode
    );
    if config.api_key.is_empty() {
        anyhow::bail!("set HI_API_KEY or run `hi config`");
    }
    let response = reqwest::Client::new()
        .get(format!("{}/models", config.base_url.trim_end_matches('/')))
        .bearer_auth(config.api_key)
        .send()
        .await
        .context("provider request failed")?;
    if !response.status().is_success() {
        anyhow::bail!("provider returned {}", response.status());
    }
    println!("provider: reachable");
    Ok(())
}
pub async fn models() -> Result<()> {
    let config = load()?;
    if config.api_key.is_empty() && config.provider != "local" {
        anyhow::bail!("set up credentials first with `hi config`");
    }
    let response = reqwest::Client::new()
        .get(format!("{}/models", config.base_url.trim_end_matches('/')))
        .bearer_auth(config.api_key)
        .send()
        .await
        .context("provider request failed")?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        anyhow::bail!("provider returned {status}: {body}");
    }
    let value: serde_json::Value =
        serde_json::from_str(&body).context("invalid models response")?;
    if let Some(models) = value.get("data").and_then(|data| data.as_array()) {
        for model in models {
            if let Some(id) = model.get("id").and_then(|id| id.as_str()) {
                println!("{id}");
            }
        }
    }
    Ok(())
}
pub fn show() {
    if let Err(error) = setup() {
        eprintln!("configuration failed: {error:#}");
    }
}
