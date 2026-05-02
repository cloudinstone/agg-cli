use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(rename = "accounts")]
    pub accounts: Vec<AccountInternal>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AccountInternal {
    pub alias: Option<String>,
    pub email: Option<String>,
    pub token_blob: String,
    pub status_blob: String,
    pub refresh_token: Option<String>,
    pub tier: Option<String>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct UserInfo {
    pub email: Option<String>,
    pub name: Option<String>,
    pub picture: Option<String>,
}

impl AppConfig {
    pub fn load() -> Self {
        let path = config_path();
        if path.exists() {
            let content = fs::read_to_string(path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else { Self::default() }
    }
    pub fn save(&self) {
        let path = config_path();
        if let Some(parent) = path.parent() { let _ = fs::create_dir_all(parent); }
        let content = serde_json::to_string_pretty(self).unwrap_or_default();
        let _ = fs::write(path, content);
    }
}

pub fn config_path() -> PathBuf {
    dirs::home_dir().unwrap().join(".aag-cli").join("accounts.json")
}

pub fn save_account(config: &mut AppConfig, account: AccountInternal) {
    if let Some(email) = &account.email {
        config.accounts.retain(|a| a.email.as_ref() != Some(email));
    }
    if let Some(alias) = &account.alias {
        config.accounts.retain(|a| a.alias.as_ref() != Some(alias));
    }
    config.accounts.push(account);
    config.save();
}
