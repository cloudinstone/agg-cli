mod modules;

use crate::modules::api::{
    build_auth_url, build_oauth_blob, build_status_blob, fetch_account_metadata,
    fetch_account_model_data, fetch_credit_usage_info, fetch_email_from_api, fetch_user_info,
    fetch_user_plan_info, oauth_client_config, refresh_access_token, TokenResponse, TOKEN_URL,
};
use crate::modules::config::{save_account, AccountInternal, AppConfig};
use crate::modules::db::{
    antigravity_db_path, apply_account_session, clear_antigravity_auth_state,
    get_current_ide_email, is_ide_running, restart_ide, terminate_ide,
};
use clap::{Parser, Subcommand};
use dialoguer::{theme::ColorfulTheme, Select};
use futures::future::join_all;
use std::fs;
use std::net::TcpListener;
use tabled::{settings::Style, Table, Tabled};

#[derive(Parser)]
#[command(name = "aag")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Authenticate and switch account immediately (Login + Inject + Restart)
    Login {
        /// Account alias (optional)
        alias: Option<String>,

        /// Manually specify long-term token (refresh token)
        #[arg(short, long)]
        refresh: Option<String>,

        /// Capture login state from the current IDE (no browser required)
        #[arg(short, long)]
        capture: bool,
    },
    /// Log out current IDE account (Force restart and clear session)
    Logout,

    /// Authorize new account (Authenticate and save only, no IDE state change)
    Add {
        /// Account alias (optional)
        alias: Option<String>,

        /// Manually specify long-term token (refresh token)
        #[arg(short, long)]
        refresh: Option<String>,

        /// Capture login state from the current IDE (no browser required)
        #[arg(short, long)]
        capture: bool,

        /// Force switch to this account after authorization
        #[arg(long)]
        switch: bool,
    },
    /// Delete specified account from local configuration
    Remove {
        /// Specify the Email to delete
        email: Option<String>,
    },

    /// Switch account in the local library (Hard Restart)
    Switch {
        /// Target account Email or Index (e.g., #1)
        email_or_index: Option<String>,
    },

    /// Real-time monitoring dashboard for all accounts
    List {
        /// Force refresh cloud data (skip local cache)
        #[arg(short, long)]
        refresh: bool,
    },
    /// View the account currently being used by the IDE
    Status {
        /// Print raw internal API responses for the current account
        #[arg(long)]
        internal: bool,
    },
    /// Clean up invalid accounts (Empty Email or duplicates)
    Clean,
}

#[derive(Tabled)]
pub struct ModelRow {
    #[tabled(rename = "Model")]
    pub name: String,
    #[tabled(rename = "ID")]
    pub id: String,
    #[tabled(rename = "Quota")]
    pub usage: String,
    #[tabled(rename = "Reset Time")]
    pub reset_time: String,
}

struct AccountGroup {
    email: String,
    is_active: bool,
    tier: String,
    status_detail: Option<String>,
    models: Vec<ModelRow>,
    error: Option<String>,
}

fn display_name_for_account(acc: &AccountInternal, email: &str) -> String {
    if let Some(alias) = acc.alias.as_ref() {
        format!("{} ({})", alias, email)
    } else {
        email.to_string()
    }
}

fn print_account_group(label_index: Option<usize>, display_name: &str, group: &AccountGroup) {
    let active_marker = if group.is_active {
        "\x1b[1;32m● [ACTIVE]\x1b[0m"
    } else {
        "\x1b[90m○ [IDLE]\x1b[0m"
    };
    let tier_color = if group.tier == "PRO" {
        "\x1b[1;35m"
    } else {
        "\x1b[1;36m"
    };
    let tier_label = format!("{}[{: <4}]\x1b[0m", tier_color, group.tier);
    let prefix = if let Some(idx) = label_index {
        format!("\n\x1b[1;97m#{} \x1b[0m", idx)
    } else {
        "\n".to_string()
    };

    println!(
        "{prefix}{} {}   {}",
        tier_label, display_name, active_marker
    );

    if let Some(detail) = &group.status_detail {
        println!("   \x1b[1;31m⚠️ Status: {}\x1b[0m", detail);
    }
    if let Some(err) = &group.error {
        println!("   \x1b[31m❌ Error: {}\x1b[0m", err);
    }

    if !group.models.is_empty() {
        let mut table_rows = Vec::new();
        #[derive(Tabled)]
        struct VisualModelRow {
            #[tabled(rename = "Model")]
            name: String,
            #[tabled(rename = "Quota")]
            progress: String,
            #[tabled(rename = "Reset Time")]
            reset: String,
        }
        for m in &group.models {
            let pct = if m.usage == "N/A" {
                0.0
            } else {
                m.usage.replace("%", "").parse::<f64>().unwrap_or(0.0)
            };
            let bar_len = 10;
            let filled = (pct / 100.0 * bar_len as f64).round() as usize;
            let bar = format!(
                "\x1b[32m{}\x1b[90m{}\x1b[0m",
                "█".repeat(filled),
                "█".repeat(bar_len - filled)
            );
            let usage_display = if m.usage == "N/A" { " N/A" } else { &m.usage };
            table_rows.push(VisualModelRow {
                name: m.name.clone(),
                progress: format!("{} {:>4}", bar, usage_display),
                reset: m.reset_time.clone(),
            });
        }
        let mut t = Table::new(table_rows);
        t.with(Style::psql());
        println!("{}", t);
    } else if group.error.is_none() {
        println!("   \x1b[90mℹ️ No data available.\x1b[0m");
    }
}

async fn perform_add_or_login(
    config: &mut AppConfig,
    refresh: Option<String>,
    capture: bool,
    alias: Option<String>,
    switch: bool,
) {
    if capture {
        let email = get_current_ide_email();
        if let Some(em) = email {
            let db_path = antigravity_db_path();
            let temp_db = std::env::temp_dir().join("aag_add.db");
            if fs::copy(&db_path, &temp_db).is_ok() {
                if let Ok(conn) = rusqlite::Connection::open(&temp_db) {
                    let (mut t_blob, mut s_blob) = (String::new(), String::new());
                    if let Ok(mut stmt) = conn.prepare("SELECT key, value FROM itemTable WHERE key IN ('antigravityUnifiedStateSync.oauthToken', 'antigravityUnifiedStateSync.userStatus')") {
                        if let Ok(mut rows) = stmt.query([]) {
                            while let Some(row) = rows.next().unwrap_or(None) {
                                let k: String = row.get(0).unwrap_or_default();
                                let v: String = row.get(1).unwrap_or_default();
                                if k.contains("oauthToken") { t_blob = v; } else if k.contains("userStatus") { s_blob = v; }
                            }
                        }
                    }
                    let _ = fs::remove_file(temp_db);

                    let account = AccountInternal {
                        alias: alias.clone(),
                        email: Some(em.clone()),
                        token_blob: t_blob,
                        status_blob: s_blob,
                        refresh_token: None,
                        tier: None,
                    };
                    save_account(config, account);
                    println!(
                        "✅ Successfully captured and saved account from IDE: {}{}",
                        em,
                        if let Some(a) = alias {
                            format!(" (Alias: {})", a)
                        } else {
                            "".to_string()
                        }
                    );
                    return;
                }
            }
        }
        println!(
            "❌ Could not detect a valid Email from current IDE. Please log in to the IDE first."
        );
        return;
    }

    if let Some(rf) = refresh {
        println!("🚀 Verifying manually entered token...");
        if let Some(token) = refresh_access_token(&rf).await {
            if let Some(user_info) = fetch_user_info(&token.access_token).await {
                if let (Some(email), Some(status_blob)) =
                    (user_info.email.clone(), build_status_blob(&user_info))
                {
                    let token_blob = build_oauth_blob(&token.access_token, &rf, token.expires_in);
                    let account = AccountInternal {
                        alias: alias.clone(),
                        email: Some(email.clone()),
                        token_blob: token_blob.clone(),
                        status_blob: status_blob.clone(),
                        refresh_token: Some(rf),
                        tier: None,
                    };
                    save_account(config, account);
                    println!("✅ Token verified successfully, account saved: {}", email);
                    return;
                }
            }
        }
        println!("❌ Token verification failed, please check if it is correct.");
        return;
    }

    // Default flow: Google OAuth
    let oauth = match oauth_client_config() {
        Some(config) => config,
        None => {
            eprintln!("❌ Could not discover Antigravity OAuth client configuration from the local app bundle.");
            return;
        }
    };
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("❌ Failed to bind local OAuth callback port: {}", e);
            return;
        }
    };
    let port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(e) => {
            eprintln!("❌ Failed to inspect local OAuth callback port: {}", e);
            return;
        }
    };
    let redirect_uri = format!("http://localhost:{}/oauth-callback", port);
    let auth_url = build_auth_url(&oauth.client_id, &redirect_uri);
    println!("🚀 Launching browser for Google Authorization...");
    let _ = webbrowser::open(&auth_url);
    if let Ok(server) = tiny_http::Server::from_listener(listener, None) {
        if let Some(request) = server.incoming_requests().next() {
            let url = url::Url::parse(&format!("http://localhost{}", request.url())).unwrap();
            if let Some(c) = url
                .query_pairs()
                .find(|(k, _)| k == "code")
                .map(|(_, v)| v.into_owned())
            {
                let _ = request.respond(tiny_http::Response::from_string(
                    "<h1>Login successful, please return to the terminal.</h1>",
                ));
                let client = reqwest::Client::new();
                let body = format!(
                    "client_id={}&client_secret={}&code={}&grant_type=authorization_code&redirect_uri={}",
                    oauth.client_id, oauth.client_secret, c, redirect_uri
                );
                let res = client
                    .post(TOKEN_URL)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(body)
                    .send()
                    .await;
                if let Ok(r) = res {
                    if let Ok(tr) = r.json::<TokenResponse>().await {
                        if let Some(refresh) = tr.refresh_token {
                            if let Some(user_info) = fetch_user_info(&tr.access_token).await {
                                if let (Some(email), Some(status_blob)) =
                                    (user_info.email.clone(), build_status_blob(&user_info))
                                {
                                    let token_blob =
                                        build_oauth_blob(&tr.access_token, &refresh, tr.expires_in);
                                    let account = AccountInternal {
                                        alias: alias.clone(),
                                        email: Some(email.clone()),
                                        token_blob: token_blob.clone(),
                                        status_blob: status_blob.clone(),
                                        refresh_token: Some(refresh),
                                        tier: None,
                                    };
                                    save_account(config, account);

                                    if switch {
                                        let user_plan =
                                            fetch_user_plan_info(&tr.access_token).await;
                                        let project_id = user_plan
                                            .get("cloudaicompanionProject")
                                            .and_then(|p| {
                                                p.as_str().map(|s| s.to_string()).or_else(|| {
                                                    p.get("id")
                                                        .and_then(|id| id.as_str())
                                                        .map(|s| s.to_string())
                                                })
                                            });
                                        let credit_usage = if let Some(pid) = project_id {
                                            fetch_credit_usage_info(&tr.access_token, &pid).await
                                        } else {
                                            serde_json::json!({})
                                        };
                                        let user_info_json = serde_json::to_value(&user_info)
                                            .unwrap_or(serde_json::json!({}));

                                        if apply_account_session(
                                            &token_blob,
                                            &status_blob,
                                            &user_info_json,
                                            &user_plan,
                                            &credit_usage,
                                        ) {
                                            terminate_ide();
                                            println!("🎉 Authorization successful! Saved and switched to '{}'.", email);
                                            restart_ide();
                                        } else {
                                            println!("🎉 Authorization successful! Account '{}' saved, but IDE injection failed.", email);
                                        }
                                    } else {
                                        println!("✅ Authorization successful! Account '{}' saved. Use `aag switch` to change to this account.", email);
                                    }
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!("❌ Authorization input failed.");
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let mut config = AppConfig::load();
    let current_email = get_current_ide_email();

    match cli.command {
        Commands::List { refresh } => {
            if config.accounts.is_empty() {
                println!("💡 Please use `aag login` first.");
                return;
            }

            println!("\n📦 Antigravity Real-time Usage Dashboard");
            let mut tasks = Vec::new();
            for (idx, acc) in config.accounts.iter().enumerate() {
                let (rf_token, email_cached) = (acc.refresh_token.clone(), acc.email.clone());
                let active_em = current_email.clone();
                let force_refresh = refresh;
                tasks.push(async move {
                    let mut email = email_cached
                        .clone()
                        .unwrap_or_else(|| "Unknown".to_string());
                    let mut models = Vec::new();
                    let mut error = None;
                    let mut tier = "FREE".to_string();
                    let mut status_detail = None;

                    if let Some(rf) = rf_token {
                        if let Some(token) = refresh_access_token(&rf).await {
                            if email == "Unknown" {
                                if let Some(e) = fetch_email_from_api(&token.access_token).await {
                                    email = e;
                                }
                            }
                            if let Some(meta) = fetch_account_metadata(&token.access_token).await {
                                tier = meta.tier;
                                if meta.validation_required {
                                    status_detail =
                                        Some("Needs validation (Verify on device)".to_string());
                                }
                                let project_id = meta.project_id.as_deref().unwrap_or("");
                                let (m_rows, _) = fetch_account_model_data(
                                    &token.access_token,
                                    project_id,
                                    &email,
                                    force_refresh,
                                )
                                .await;
                                models = m_rows;
                            } else {
                                error = Some("Handshake Fail".into());
                            }
                        } else {
                            error = Some("Refresh Fail".into());
                        }
                    } else {
                        status_detail = Some("Session-only account".into());
                    }

                    let is_active = active_em.as_ref() == Some(&email);
                    (
                        idx,
                        AccountGroup {
                            email,
                            is_active,
                            tier,
                            status_detail,
                            models,
                            error,
                        },
                    )
                });
            }

            let mut results = join_all(tasks).await;
            results.sort_by(|(_, a), (_, b)| {
                let a_available =
                    a.status_detail.as_deref() != Some("Needs validation (Verify on device)");
                let b_available =
                    b.status_detail.as_deref() != Some("Needs validation (Verify on device)");

                if a_available != b_available {
                    b_available.cmp(&a_available)
                } else {
                    a.email.to_lowercase().cmp(&b.email.to_lowercase())
                }
            });

            for (idx, group) in results {
                let display_name = display_name_for_account(&config.accounts[idx], &group.email);
                print_account_group(Some(idx + 1), &display_name, &group);
            }
        }
        Commands::Status { internal } => {
            let Some(em) = current_email.clone() else {
                println!("\nℹ️ Could not detect current account.\n");
                return;
            };

            let Some(acc) = config
                .accounts
                .iter()
                .find(|a| a.email.as_deref() == Some(em.as_str()))
            else {
                println!("\n✨ Current IDE is using: {}\n", em);
                println!("ℹ️ This account is not in the local account library, so detailed status is unavailable.\n");
                return;
            };

            let mut group = AccountGroup {
                email: em.clone(),
                is_active: true,
                tier: "FREE".to_string(),
                status_detail: None,
                models: Vec::new(),
                error: None,
            };
            let mut raw_plan: Option<serde_json::Value> = None;
            let mut raw_models: Option<serde_json::Value> = None;

            if let Some(rf) = acc.refresh_token.as_deref() {
                if let Some(token) = refresh_access_token(rf).await {
                    if let Some(meta) = fetch_account_metadata(&token.access_token).await {
                        group.tier = meta.tier;
                        if meta.validation_required {
                            group.status_detail =
                                Some("Needs validation (Verify on device)".to_string());
                        }
                        let project_id = meta.project_id.unwrap_or_default();
                        let (models, _) =
                            fetch_account_model_data(&token.access_token, &project_id, &em, false)
                                .await;
                        group.models = models;
                        if internal {
                            raw_plan = Some(fetch_user_plan_info(&token.access_token).await);
                            raw_models = Some(
                                fetch_credit_usage_info(&token.access_token, &project_id).await,
                            );
                        }
                    } else {
                        group.error = Some("Handshake Fail".into());
                    }
                } else {
                    group.error = Some("Refresh Fail".into());
                }
            } else {
                group.status_detail = Some("Session-only account".into());
            }

            let display_name = display_name_for_account(acc, &em);
            print_account_group(None, &display_name, &group);

            if internal {
                if let (Some(plan), Some(models)) = (raw_plan, raw_models) {
                    println!("\n**loadCodeAssist**");
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&plan).unwrap_or_else(|_| "{}".to_string())
                    );
                    println!("\n**fetchAvailableModels**");
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&models).unwrap_or_else(|_| "{}".to_string())
                    );
                } else {
                    println!(
                        "\nℹ️ `--internal` requires a saved refresh token for the current account."
                    );
                }
            }
        }
        Commands::Add {
            alias,
            refresh,
            capture,
            switch,
        } => {
            perform_add_or_login(&mut config, refresh, capture, alias, switch).await;
        }
        Commands::Login {
            alias,
            refresh,
            capture,
        } => {
            perform_add_or_login(&mut config, refresh, capture, alias, true).await;
        }
        Commands::Logout => {
            println!("🚀 Clearing current login state...");
            let was_running = terminate_ide();
            clear_antigravity_auth_state();
            println!("🎉 Logout complete! Login state cleared and running environment closed.");
            if was_running {
                restart_ide();
            }
        }
        Commands::Switch { email_or_index } => {
            if config.accounts.is_empty() {
                println!("💡 No accounts available. Please use `aag login`.");
                return;
            }

            println!("⏳ Fetching real-time account status and quotas...");
            let mut tasks = Vec::new();
            for (idx, acc) in config.accounts.iter().enumerate() {
                let acc = acc.clone();
                tasks.push(tokio::spawn(async move {
                    let mut tier = "FREE".to_string();
                    let mut error = None;
                    let mut needs_validation = false;
                    let mut display_email =
                        acc.email.clone().unwrap_or_else(|| "Unknown".to_string());
                    let mut gemini_quota = "N/A".to_string();
                    let mut claude_quota = "N/A".to_string();

                    if let Some(rf) = &acc.refresh_token {
                        if let Some(token) = refresh_access_token(rf).await {
                            if let Some(meta) = fetch_account_metadata(&token.access_token).await {
                                tier = meta.tier.clone();
                                needs_validation = meta.validation_required;
                                let mut user_email = display_email.clone();
                                if let Some(user) = fetch_user_info(&token.access_token).await {
                                    if let Some(e) = user.email {
                                        user_email = e.clone();
                                        display_email = e;
                                    }
                                }

                                let project_id = meta.project_id.as_deref().unwrap_or("");
                                let (models, _) = fetch_account_model_data(
                                    &token.access_token,
                                    project_id,
                                    &user_email,
                                    true,
                                )
                                .await;
                                for m in models {
                                    if m.id == "gemini-3.1-pro-high" {
                                        gemini_quota = m.usage;
                                    } else if m.id == "claude-opus-4-6-thinking" {
                                        claude_quota = m.usage;
                                    }
                                }
                            } else {
                                error = Some("Handshake Fail".to_string());
                            }
                        } else {
                            error = Some("Token Expired".to_string());
                        }
                    } else {
                        tier = "SESSION".to_string();
                    }

                    (
                        idx,
                        display_email,
                        tier,
                        needs_validation,
                        error,
                        gemini_quota,
                        claude_quota,
                    )
                }));
            }

            let results = join_all(tasks).await;
            let mut status_map = std::collections::HashMap::new();

            struct MenuItem {
                idx: usize,
                display_name: String,
                gemini_quota: String,
                claude_quota: String,
                sort_key: String,
                is_available: bool,
                status_label: String,
                tier: String,
            }
            let mut items_data = Vec::new();

            for (idx, email, tier, needs_val, err, gq, cq) in results.into_iter().flatten() {
                let is_err = err.is_some();
                let is_available = !needs_val;

                let status_label = if needs_val {
                    "\x1b[33m⚠️  Validation Required\x1b[0m".to_string()
                } else if is_err {
                    "\x1b[31m❌ Error\x1b[0m".to_string()
                } else if tier == "SESSION" {
                    "\x1b[90mℹ Session only\x1b[0m".to_string()
                } else {
                    "\x1b[32m✅ Ready\x1b[0m".to_string()
                };

                items_data.push(MenuItem {
                    idx,
                    display_name: if let Some(alias) = config.accounts[idx].alias.as_ref() {
                        format!("{} ({})", alias, email)
                    } else {
                        email.clone()
                    },
                    gemini_quota: gq,
                    claude_quota: cq,
                    sort_key: email.to_lowercase(),
                    is_available,
                    status_label,
                    tier,
                });
                status_map.insert(idx, (needs_val, is_err));
            }

            items_data.sort_by(|a, b| {
                if a.is_available != b.is_available {
                    b.is_available.cmp(&a.is_available)
                } else {
                    a.sort_key.cmp(&b.sort_key)
                }
            });

            let menu_items: Vec<String> = items_data
                .iter()
                .map(|item| {
                    let format_quota = |label: &str, q: &str| {
                        let q_val = if q == "N/A" { " N/A" } else { q };
                        let color = if q == "N/A" {
                            "\x1b[90m"
                        } else if q.replace("%", "").parse::<f64>().unwrap_or(0.0) > 50.0 {
                            "\x1b[32m"
                        } else {
                            "\x1b[31m"
                        };

                        let label_color = if label == "G" {
                            "\x1b[1;36m"
                        } else {
                            "\x1b[1;33m"
                        };
                        format!(
                            "{}{}\x1b[0m {}{:>4}\x1b[0m",
                            label_color, label, color, q_val
                        )
                    };

                    let g_fmt = format_quota("G", &item.gemini_quota);
                    let c_fmt = format_quota("C", &item.claude_quota);

                    let name_len = item.display_name.chars().count();
                    let dots1_count = if name_len < 38 { 38 - name_len } else { 1 };
                    let dots1 = "\x1b[90m".to_owned() + &".".repeat(dots1_count) + "\x1b[0m";

                    let quota_part = format!("{}  {}", g_fmt, c_fmt);
                    let dots2 = "\x1b[90m........\x1b[0m";

                    let tier_part = if item.tier == "SESSION" {
                        "\x1b[90m[SESSION]\x1b[0m".to_string()
                    } else {
                        let tier_color = if item.tier == "PRO" {
                            "\x1b[1;35m"
                        } else {
                            "\x1b[1;36m"
                        };
                        format!("{}[{: <4}]\x1b[0m", tier_color, item.tier)
                    };

                    format!(
                        "{} {} {} {} {} {}",
                        tier_part, item.display_name, dots1, quota_part, dots2, item.status_label
                    )
                })
                .collect();

            let index_mapping: Vec<usize> = items_data.iter().map(|item| item.idx).collect();

            let acc_index = if let Some(input) = email_or_index {
                if let Some(stripped) = input.strip_prefix('#') {
                    let idx = stripped.parse::<usize>().unwrap_or(0);
                    if idx > 0 && idx <= config.accounts.len() {
                        idx - 1
                    } else {
                        println!("❌ Invalid index");
                        return;
                    }
                } else {
                    match config.accounts.iter().position(|a| {
                        a.email.as_ref() == Some(&input) || a.alias.as_ref() == Some(&input)
                    }) {
                        Some(i) => i,
                        None => {
                            println!("❌ Could not find account '{}'", input);
                            return;
                        }
                    }
                }
            } else {
                match Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("Please select an account to switch to")
                    .default(0)
                    .items(&menu_items)
                    .interact_opt()
                {
                    Ok(Some(sel_idx)) => index_mapping[sel_idx],
                    Ok(None) => {
                        println!("❌ Cancelled.");
                        return;
                    }
                    Err(_) => {
                        println!("❌ Cancelled.");
                        return;
                    }
                }
            };

            if let Some((needs_val, _)) = status_map.get(&acc_index) {
                if *needs_val {
                    println!("\n❌ Switch failed: This account requires secondary validation. Please log in via browser or run `aag login`.\n");
                    return;
                }
            }

            let acc = &config.accounts[acc_index];
            let mut token_blob = acc.token_blob.clone();
            let mut status_blob = acc.status_blob.clone();
            let mut resolved_email = acc.email.clone();

            println!(
                "🚀 Switching to account: {}...",
                resolved_email.as_deref().unwrap_or("Unknown")
            );

            let was_running = is_ide_running();

            if let Some(refresh_token) = &acc.refresh_token {
                if let Some(token) = refresh_access_token(refresh_token).await {
                    token_blob =
                        build_oauth_blob(&token.access_token, refresh_token, token.expires_in);
                    if let Some(user_info) = fetch_user_info(&token.access_token).await {
                        if let Some(email) = user_info.email.clone() {
                            resolved_email = Some(email);
                            if let Some(new_status_blob) = build_status_blob(&user_info) {
                                status_blob = new_status_blob;
                            }
                        }
                        let user_plan = fetch_user_plan_info(&token.access_token).await;
                        let project_id = user_plan.get("cloudaicompanionProject").and_then(|p| {
                            p.as_str().map(|s| s.to_string()).or_else(|| {
                                p.get("id")
                                    .and_then(|id| id.as_str())
                                    .map(|s| s.to_string())
                            })
                        });
                        let credit_usage = if let Some(pid) = project_id {
                            fetch_credit_usage_info(&token.access_token, &pid).await
                        } else {
                            serde_json::json!({})
                        };
                        let user_info_json =
                            serde_json::to_value(&user_info).unwrap_or(serde_json::json!({}));

                        if apply_account_session(
                            &token_blob,
                            &status_blob,
                            &user_info_json,
                            &user_plan,
                            &credit_usage,
                        ) {
                            if let Some(account) = config.accounts.get_mut(acc_index) {
                                account.token_blob = token_blob;
                                account.status_blob = status_blob;
                                if resolved_email.is_some() {
                                    account.email = resolved_email.clone();
                                }
                            }
                            config.save();

                            if was_running {
                                let restarted = terminate_ide();
                                if restarted {
                                    restart_ide();
                                }
                            }
                            println!("🎉 Switch successful!");
                            return;
                        }
                    }
                }
                if apply_account_session(
                    &token_blob,
                    &status_blob,
                    &serde_json::json!({}),
                    &serde_json::json!({}),
                    &serde_json::json!({}),
                ) {
                    if was_running {
                        let restarted = terminate_ide();
                        if restarted {
                            restart_ide();
                        }
                    }
                    println!("⚠️ Switched using cached local session data. Token refresh failed, so the IDE may still require re-authentication.");
                    return;
                }
                eprintln!(
                    "❌ Failed to refresh this account and could not write cached session data."
                );
                return;
            }

            if apply_account_session(
                &token_blob,
                &status_blob,
                &serde_json::json!({}),
                &serde_json::json!({}),
                &serde_json::json!({}),
            ) {
                if was_running {
                    let restarted = terminate_ide();
                    if restarted {
                        restart_ide();
                    }
                }
                println!("🎉 Switch successful! (Session-only account)");
            } else {
                eprintln!("❌ Failed to write login state.");
            }
        }
        Commands::Remove { email } => {
            if let Some(e) = email {
                let original_len = config.accounts.len();
                config
                    .accounts
                    .retain(|a| a.email.as_ref() != Some(&e) && a.alias.as_ref() != Some(&e));
                if config.accounts.len() < original_len {
                    config.save();
                    println!("✅ Deleted account: {}", e);
                } else {
                    println!("❌ Could not find account: {}", e);
                }
            } else {
                if config.accounts.is_empty() {
                    println!("💡 No accounts available.");
                    return;
                }
                let items: Vec<String> = config
                    .accounts
                    .iter()
                    .map(|acc| {
                        acc.email
                            .as_deref()
                            .unwrap_or("Unknown Email (Run `aag clean`)")
                            .to_string()
                    })
                    .collect();
                match Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("Please select an account to delete")
                    .default(0)
                    .items(&items)
                    .interact_opt()
                {
                    Ok(Some(index)) => {
                        let removed = config.accounts.remove(index);
                        config.save();
                        println!(
                            "✅ Deleted account: {}",
                            removed.email.unwrap_or_else(|| "Unknown".to_string())
                        );
                    }
                    Ok(None) => {
                        println!("❌ Cancelled.");
                        return;
                    }
                    Err(_) => {
                        println!("❌ Cancelled.");
                        return;
                    }
                }
            }
        }
        Commands::Clean => {
            let original_len = config.accounts.len();
            config.accounts.retain(|a| a.email.is_some());
            let mut seen = std::collections::HashSet::new();
            let mut unique_accounts = Vec::new();
            for acc in config.accounts.into_iter().rev() {
                if let Some(ref email) = acc.email {
                    if !seen.contains(email) {
                        seen.insert(email.clone());
                        unique_accounts.push(acc);
                    }
                }
            }
            config.accounts = unique_accounts.into_iter().rev().collect();
            let removed_count = original_len - config.accounts.len();
            if removed_count > 0 {
                config.save();
                println!(
                    "🧹 Cleanup complete! Removed {} invalid or duplicate entries.",
                    removed_count
                );
            } else {
                println!("✨ List is already clean.");
            }
        }
    }
}
