use anyhow::Result;
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
    migrate(&mut config);
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

/// Bring old or partially-written configs in line with the provider registry:
/// legacy provider names map to their modern equivalents, and untouched
/// OpenAI-style defaults are replaced with the selected provider's presets.
fn migrate(config: &mut Config) {
    if config.provider == "openai-compatible" {
        config.provider = "openai".into();
    }
    if config.provider == "local" {
        config.provider = "ollama".into();
    }
    if config.provider == "openai" {
        return;
    }
    let Some((url, model, _)) = crate::providers::preset(&config.provider) else {
        return;
    };
    if config.base_url.is_empty() || config.base_url == Config::default().base_url {
        config.base_url = url.into();
    }
    if config.model == "gpt-4o-mini" {
        config.model = model.into();
    }
}

/// True when the provider needs an API key; local providers like Ollama do not.
pub fn key_required(provider: &str) -> bool {
    crate::providers::preset(provider)
        .map(|(_, _, requires)| requires)
        .unwrap_or(true)
}

pub fn setup() -> Result<()> {
    let old = load()?;
    println!("\nhi setup\n=======");
    println!("Choose the provider for this installation:");
    let providers = crate::providers::provider_names();
    for (index, name) in providers.iter().enumerate() {
        println!("  {}) {}", index + 1, provider_display(name));
    }
    let default_choice = providers
        .iter()
        .position(|name| *name == old.provider)
        .map(|index| (index + 1).to_string())
        .unwrap_or_else(|| "1".into());
    let choice = prompt("Provider", &default_choice)?;
    let (provider, default_url, key_required): (String, String, bool) =
        match choice.trim().parse::<usize>() {
            Ok(index) if index >= 1 && index <= providers.len() => {
                let name = providers[index - 1];
                let (url, _, requires_key) =
                    crate::providers::preset(name).unwrap_or(("", "", false));
                (name.to_string(), url.into(), requires_key)
            }
            _ => {
                let (url, _, requires_key) = crate::providers::preset(&old.provider).unwrap_or((
                    "https://api.openai.com/v1",
                    "gpt-4o-mini",
                    true,
                ));
                (old.provider.clone(), url.into(), requires_key)
            }
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
    let base_url = if provider.as_str() == "openai" {
        default_url
    } else {
        prompt("Base URL", &default_url)?
    };
    println!("\nStep 3 of 3: model");
    let (_, preset_model, _) =
        crate::providers::preset(&provider).unwrap_or(("", "gpt-4o-mini", false));
    let default_model = if old.provider == provider && !old.model.is_empty() {
        old.model.clone()
    } else {
        preset_model.into()
    };
    let model = prompt("Model ID", &default_model)?;
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
fn provider_display(name: &str) -> String {
    match name {
        "openai" => "OpenAI".into(),
        "anthropic" => "Anthropic (Claude)".into(),
        "gemini" => "Google Gemini".into(),
        "openrouter" => "OpenRouter".into(),
        "ollama" => "Ollama (local, no API key)".into(),
        other => other.to_string(),
    }
}
pub fn ensure_configured() -> Result<()> {
    let config = load()?;
    if !config_path().exists() || (config.api_key.is_empty() && key_required(&config.provider)) {
        println!("Welcome to hi. Let's configure your AI provider first.");
        setup()?;
        let configured = load()?;
        if configured.api_key.is_empty() && key_required(&configured.provider) {
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
    if config.api_key.is_empty() && key_required(&config.provider) {
        anyhow::bail!("set HI_API_KEY or run `hi config`");
    }
    let provider = crate::providers::create(&config)?;
    match provider.models().await {
        Ok(models) => println!("provider: reachable ({} models exposed)", models.len()),
        Err(error) => {
            anyhow::bail!("provider unreachable: {error:#}");
        }
    }
    Ok(())
}
pub async fn models() -> Result<()> {
    let config = load()?;
    if config.api_key.is_empty() && key_required(&config.provider) {
        anyhow::bail!("set up credentials first with `hi config`");
    }
    let provider = crate::providers::create(&config)?;
    let list = provider.models().await?;
    for id in list {
        println!("{id}");
    }
    Ok(())
}
pub fn show() {
    if let Err(error) = setup() {
        eprintln!("configuration failed: {error:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(provider: &str) -> Config {
        Config {
            provider: provider.into(),
            api_key: String::new(),
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-4o-mini".into(),
            approval_mode: "always".into(),
        }
    }

    #[test]
    fn migrates_legacy_provider_names() {
        let mut compatible = sample("openai-compatible");
        compatible.base_url = "https://example.com/v1".into();
        migrate(&mut compatible);
        assert_eq!(compatible.provider, "openai");
        assert_eq!(compatible.base_url, "https://example.com/v1");

        let mut local = sample("local");
        local.base_url = "http://localhost:11434/v1".into();
        migrate(&mut local);
        assert_eq!(local.provider, "ollama");
        assert_eq!(local.base_url, "http://localhost:11434/v1");
    }

    #[test]
    fn applies_presets_when_openai_defaults_untouched() {
        let mut anthropic = sample("anthropic");
        migrate(&mut anthropic);
        assert_eq!(anthropic.base_url, "https://api.anthropic.com");
        assert_eq!(anthropic.model, crate::providers::anthropic::PRESET_MODEL);

        let mut ollama = sample("ollama");
        migrate(&mut ollama);
        assert_eq!(ollama.base_url, "http://localhost:11434");
        assert_eq!(ollama.model, crate::providers::ollama::PRESET_MODEL);
    }

    #[test]
    fn openai_preserves_custom_base_url() {
        let mut custom = sample("openai");
        custom.base_url = "https://my-gateway.example/v2".into();
        migrate(&mut custom);
        assert_eq!(custom.base_url, "https://my-gateway.example/v2");
        assert_eq!(custom.model, "gpt-4o-mini");
    }
}
