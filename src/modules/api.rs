use crate::modules::config::UserInfo;
use crate::ModelRow;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
pub const BASE_URL: &str = "https://cloudcode-pa.googleapis.com";

const OAUTH_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";
const APP_MAIN_JS_PATH: &str = "/Applications/Antigravity.app/Contents/Resources/app/out/main.js";

#[derive(Clone, Debug)]
pub struct OAuthClientConfig {
    pub client_id: String,
    pub client_secret: String,
}

static OAUTH_CLIENT_CONFIG: OnceLock<Option<OAuthClientConfig>> = OnceLock::new();

#[derive(serde::Deserialize, Debug)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<i64>,
}

#[derive(Debug)]
pub struct AccountMetadata {
    pub project_id: Option<String>,
    pub tier: String,
    pub validation_required: bool,
}

pub fn oauth_client_config() -> Option<&'static OAuthClientConfig> {
    OAUTH_CLIENT_CONFIG
        .get_or_init(discover_oauth_client_config)
        .as_ref()
}

fn discover_oauth_client_config() -> Option<OAuthClientConfig> {
    discover_oauth_client_config_from_main_js()
}

fn discover_oauth_client_config_from_main_js() -> Option<OAuthClientConfig> {
    let content = fs::read_to_string(APP_MAIN_JS_PATH).ok()?;
    let pattern =
        regex::Regex::new(r#"kfe="([^"]+apps\.googleusercontent\.com)",_fe="([^"]+)""#).ok()?;
    let captures = pattern.captures(&content)?;
    Some(OAuthClientConfig {
        client_id: captures.get(1)?.as_str().to_string(),
        client_secret: captures.get(2)?.as_str().to_string(),
    })
}

pub fn build_auth_url(client_id: &str, redirect_uri: &str) -> String {
    format!(
        "https://accounts.google.com/o/oauth2/v2/auth?access_type=offline&prompt=consent&scope={}&response_type=code&client_id={}&redirect_uri={}",
        urlencoding::encode(OAUTH_SCOPE),
        client_id,
        urlencoding::encode(redirect_uri),
    )
}

pub fn cache_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".aag-cli").join("cache")
}

pub async fn refresh_access_token(refresh_token: &str) -> Option<TokenResponse> {
    let oauth = oauth_client_config()?;
    let client = reqwest::Client::new();
    let body = format!(
        "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
        oauth.client_id, oauth.client_secret, refresh_token
    );
    let res = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .ok()?;
    res.json::<TokenResponse>().await.ok()
}

pub async fn fetch_user_info(access_token: &str) -> Option<UserInfo> {
    let client = reqwest::Client::new();
    let res = client
        .get(USERINFO_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .ok()?;
    res.json::<UserInfo>().await.ok()
}

pub async fn fetch_email_from_api(access_token: &str) -> Option<String> {
    fetch_user_info(access_token).await.and_then(|u| u.email)
}

pub async fn fetch_account_metadata(access_token: &str) -> Option<AccountMetadata> {
    let client = reqwest::Client::new();
    let payload = serde_json::json!({ "metadata": { "ideName": "antigravity", "ideType": "ANTIGRAVITY", "ideVersion": "1.20.5" }, "mode": "FULL_ELIGIBILITY_CHECK" });
    let res = client
        .post(format!("{}/v1internal:loadCodeAssist", BASE_URL))
        .bearer_auth(access_token)
        .header("User-Agent", "antigravity/1.20.5 darwin/arm64")
        .json(&payload)
        .send()
        .await
        .ok()?;

    let v: serde_json::Value = res.json().await.ok()?;
    let project_id = v.get("cloudaicompanionProject").and_then(|p| {
        p.as_str().map(|s| s.to_string()).or_else(|| {
            p.get("id")
                .and_then(|id| id.as_str())
                .map(|s| s.to_string())
        })
    });

    let tier_id = v
        .get("currentTier")
        .and_then(|t| t.get("id"))
        .and_then(|id| id.as_str())
        .unwrap_or("free-tier");
    let tier = if tier_id.contains("pro") || v.get("paidTier").is_some() {
        "PRO"
    } else {
        "FREE"
    };

    let validation_required = v
        .get("ineligibleTiers")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter().any(|item| {
                item.get("reasonCode").and_then(|r| r.as_str()) == Some("VALIDATION_REQUIRED")
            })
        })
        .unwrap_or(false);

    Some(AccountMetadata {
        project_id,
        tier: tier.to_string(),
        validation_required,
    })
}

pub async fn fetch_account_model_data(
    access_token: &str,
    project_id: &str,
    email: &str,
    force_refresh: bool,
) -> (Vec<ModelRow>, Vec<String>) {
    let cache_file = cache_dir().join(format!("{}.json", email));
    let mut raw_json: Option<serde_json::Value> = None;

    if !force_refresh && cache_file.exists() {
        if let Ok(content) = fs::read_to_string(&cache_file) {
            raw_json = serde_json::from_str(&content).ok();
        }
    }

    if raw_json.is_none() {
        let client = reqwest::Client::new();
        let payload = serde_json::json!({ "project": project_id });
        let res = client
            .post(format!("{}/v1internal:fetchAvailableModels", BASE_URL))
            .bearer_auth(access_token)
            .header("User-Agent", "antigravity/1.20.5 darwin/arm64")
            .json(&payload)
            .send()
            .await
            .ok();

        if let Some(resp) = res {
            if let Ok(v) = resp.json::<serde_json::Value>().await {
                let _ = fs::create_dir_all(cache_dir());
                let _ = fs::write(
                    &cache_file,
                    serde_json::to_string_pretty(&v).unwrap_or_default(),
                );
                raw_json = Some(v);
            }
        }
    }

    let mut rows = Vec::new();
    let mut recommended_ids = Vec::new();

    if let Some(v) = raw_json {
        if let Some(sorts) = v.get("agentModelSorts").and_then(|s| s.as_array()) {
            if let Some(first_sort) = sorts.first() {
                if let Some(groups) = first_sort.get("groups").and_then(|g| g.as_array()) {
                    if let Some(first_group) = groups.first() {
                        if let Some(ids) = first_group.get("modelIds").and_then(|i| i.as_array()) {
                            recommended_ids = ids
                                .iter()
                                .filter_map(|id| id.as_str().map(|s| s.to_string()))
                                .collect();
                        }
                    }
                }
            }
        }
        if recommended_ids.is_empty() {
            recommended_ids = vec![
                "gemini-3.1-pro-high".into(),
                "gemini-3.1-pro-low".into(),
                "gemini-3-flash-agent".into(),
                "claude-sonnet-4-6".into(),
                "claude-opus-4-6-thinking".into(),
                "gpt-oss-120b-medium".into(),
            ];
        }

        if let Some(models) = v.get("models").and_then(|m| m.as_object()) {
            for id in &recommended_ids {
                if let Some(detail) = models.get(id) {
                    let display_name = detail
                        .get("displayName")
                        .and_then(|n| n.as_str())
                        .unwrap_or(id);
                    let mut usage = "N/A".to_string();
                    let mut reset_time = "-".to_string();
                    if let Some(qi) = detail.get("quotaInfo") {
                        if let Some(rem) = qi
                            .get("remainingFraction")
                            .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                        {
                            usage = format!("{:.0}%", rem * 100.0);
                            reset_time = qi
                                .get("resetTime")
                                .and_then(|t| t.as_str())
                                .unwrap_or("-")
                                .replace("T", " ")
                                .replace("Z", "");
                        }
                    }
                    rows.push(ModelRow {
                        name: display_name.replace(" (Thinking)", ""),
                        id: id.clone(),
                        usage,
                        reset_time,
                    });
                }
            }
        }
    }
    (rows, recommended_ids)
}

pub async fn fetch_user_plan_info(access_token: &str) -> serde_json::Value {
    let client = reqwest::Client::new();
    let payload = serde_json::json!({ "metadata": { "ideName": "antigravity", "ideType": "ANTIGRAVITY", "ideVersion": "1.20.5" }, "mode": "FULL_ELIGIBILITY_CHECK" });
    let res = client
        .post(format!("{}/v1internal:loadCodeAssist", BASE_URL))
        .bearer_auth(access_token)
        .header("User-Agent", "antigravity/1.20.5 darwin/arm64")
        .json(&payload)
        .send()
        .await;
    if let Ok(r) = res {
        r.json().await.unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    }
}

pub async fn fetch_credit_usage_info(access_token: &str, project_id: &str) -> serde_json::Value {
    let client = reqwest::Client::new();
    let payload = serde_json::json!({ "project": project_id });
    let res = client
        .post(format!("{}/v1internal:fetchAvailableModels", BASE_URL))
        .bearer_auth(access_token)
        .header("User-Agent", "antigravity/1.20.5 darwin/arm64")
        .json(&payload)
        .send()
        .await;
    if let Ok(r) = res {
        r.json().await.unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    }
}

pub fn build_oauth_blob(
    access_token: &str,
    refresh_token: &str,
    expires_in: Option<i64>,
) -> String {
    let expires_at = SystemTime::now()
        .checked_add(Duration::from_secs(
            expires_in.unwrap_or(3600).max(60) as u64
        ))
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    crate::modules::db::create_unified_state_value(
        "oauthTokenInfoSentinelKey",
        &crate::modules::db::encode_string_field(1, access_token)
            .into_iter()
            .chain(crate::modules::db::encode_string_field(2, "Bearer"))
            .chain(crate::modules::db::encode_string_field(3, refresh_token))
            .chain(crate::modules::db::encode_len_delim_field(4, &{
                let mut t = crate::modules::db::encode_varint((1 << 3) as u64);
                t.extend(crate::modules::db::encode_varint(expires_at.max(0) as u64));
                t
            }))
            .collect::<Vec<u8>>(),
    )
}

pub fn build_status_blob(user_info: &UserInfo) -> Option<String> {
    let email = user_info.email.as_deref()?;
    let display_name = user_info
        .name
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| email.split('@').next().unwrap_or("User"));
    let mut out = Vec::new();
    out.extend(crate::modules::db::encode_string_field(3, display_name));
    out.extend(crate::modules::db::encode_string_field(7, email));
    if let Some(url) = user_info.picture.as_deref().filter(|url| !url.is_empty()) {
        out.extend(crate::modules::db::encode_string_field(38, url));
    }
    Some(crate::modules::db::create_unified_state_value(
        "userStatusSentinelKey",
        &out,
    ))
}
