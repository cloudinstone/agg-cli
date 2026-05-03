use crate::modules::crypto;
use base64::{engine::general_purpose, Engine as _};
use std::fs;
use std::path::PathBuf;

pub fn antigravity_db_path() -> PathBuf {
    dirs::home_dir()
        .unwrap()
        .join("Library/Application Support/Antigravity/User/globalStorage/state.vscdb")
}

pub fn encode_varint(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while n >= 0x80 {
        out.push((n as u8) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
    out
}

pub fn encode_len_delim_field(field_num: u32, data: &[u8]) -> Vec<u8> {
    let mut out = encode_varint(((field_num << 3) | 2) as u64);
    out.extend(encode_varint(data.len() as u64));
    out.extend_from_slice(data);
    out
}

pub fn encode_string_field(field_num: u32, value: &str) -> Vec<u8> {
    encode_len_delim_field(field_num, value.as_bytes())
}

pub fn create_unified_state_value(sentinel: &str, payload: &[u8]) -> String {
    let payload_b64 = general_purpose::STANDARD.encode(payload);
    let row = encode_string_field(1, &payload_b64);
    let inner = [
        encode_string_field(1, sentinel),
        encode_len_delim_field(2, &row),
    ]
    .concat();
    let outer = encode_len_delim_field(1, &inner);
    general_purpose::STANDARD.encode(outer)
}

fn decode_varint(bytes: &[u8], offset: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    let mut shift = 0u32;

    while *offset < bytes.len() && shift <= 63 {
        let byte = bytes[*offset];
        *offset += 1;
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
    }

    None
}

fn next_len_delimited_field<'a>(bytes: &'a [u8], offset: &mut usize) -> Option<(u32, &'a [u8])> {
    let key = decode_varint(bytes, offset)?;
    let wire_type = (key & 0x07) as u8;
    if wire_type != 2 {
        return None;
    }

    let field_num = (key >> 3) as u32;
    let len = decode_varint(bytes, offset)? as usize;
    let end = offset.checked_add(len)?;
    if end > bytes.len() {
        return None;
    }

    let data = &bytes[*offset..end];
    *offset = end;
    Some((field_num, data))
}

fn decode_unified_state_payload(value: &str) -> Option<Vec<u8>> {
    let outer_bytes = general_purpose::STANDARD.decode(value).ok()?;

    let mut outer_offset = 0usize;
    let (outer_field, inner_bytes) = next_len_delimited_field(&outer_bytes, &mut outer_offset)?;
    if outer_field != 1 {
        return None;
    }

    let mut inner_offset = 0usize;
    let mut sentinel_ok = false;
    let mut row_bytes = None;

    while inner_offset < inner_bytes.len() {
        let (field_num, data) = next_len_delimited_field(inner_bytes, &mut inner_offset)?;
        match field_num {
            1 => sentinel_ok = data == b"userStatusSentinelKey",
            2 => row_bytes = Some(data),
            _ => {}
        }
    }

    if !sentinel_ok {
        return None;
    }

    let row_bytes = row_bytes?;
    let mut row_offset = 0usize;
    let (row_field, payload_b64) = next_len_delimited_field(row_bytes, &mut row_offset)?;
    if row_field != 1 {
        return None;
    }

    let payload_b64 = std::str::from_utf8(payload_b64).ok()?;
    general_purpose::STANDARD.decode(payload_b64).ok()
}

fn decode_email_from_user_status_payload(payload: &[u8]) -> Option<String> {
    let mut offset = 0usize;

    while offset < payload.len() {
        let (field_num, data) = next_len_delimited_field(payload, &mut offset)?;
        if field_num == 7 {
            return std::str::from_utf8(data).ok().map(|s| s.to_string());
        }
    }

    None
}

pub fn clear_antigravity_auth_state() {
    let db_path = antigravity_db_path();
    if let Ok(conn) = rusqlite::Connection::open(&db_path) {
        let _ = conn.execute(
            "DELETE FROM itemTable WHERE key LIKE 'antigravityUnifiedStateSync.oauthToken%'",
            [],
        );
        let _ = conn.execute(
            "DELETE FROM itemTable WHERE key LIKE 'antigravityUnifiedStateSync.userStatus%'",
            [],
        );
        let _ = conn.execute(
            "DELETE FROM itemTable WHERE key LIKE 'secret://aicoding.auth%'",
            [],
        );
        let _ = conn.execute(
            "DELETE FROM itemTable WHERE key IN ('antigravity.profileUrl', 'antigravity.account')",
            [],
        );
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }
}

pub fn build_vscode_secret_buffer(encrypted_bytes: &[u8]) -> String {
    let buffer_json = serde_json::json!({
        "type": "Buffer",
        "data": encrypted_bytes
    });
    serde_json::to_string(&buffer_json).unwrap_or_default()
}

pub fn terminate_ide() -> bool {
    let was_running = is_ide_running();
    if was_running {
        const GRACEFUL_QUIT_POLLS: usize = 3;
        const TERM_QUIT_POLLS: usize = 12;
        const POLL_INTERVAL_MS: u64 = 500;

        println!("⏳ Closing Antigravity...");

        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg("quit app \"Antigravity\"")
            .output();

        for _ in 0..GRACEFUL_QUIT_POLLS {
            if !is_ide_running() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));
        }

        if is_ide_running() {
            println!("⏳ Requesting process shutdown...");
            let _ = std::process::Command::new("pkill")
                .arg("-TERM")
                .arg("-f")
                .arg("Antigravity.app/Contents/MacOS/Electron")
                .output();

            for _ in 0..TERM_QUIT_POLLS {
                if !is_ide_running() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));
            }
        }

        if is_ide_running() {
            println!("⏳ App is still running, falling back to force kill...");
            let _ = std::process::Command::new("pkill")
                .arg("-9")
                .arg("-f")
                .arg("Antigravity.app/Contents/MacOS/Electron")
                .output();

            let helper_patterns = ["Antigravity Helper", "language_server_macos"];
            for pattern in helper_patterns {
                let _ = std::process::Command::new("pkill")
                    .arg("-9")
                    .arg("-f")
                    .arg(pattern)
                    .output();
            }
        }
    }
    was_running
}

pub fn is_ide_running() -> bool {
    let output = std::process::Command::new("pgrep")
        .arg("-f")
        .arg("Antigravity.app/Contents/MacOS/Electron")
        .output();

    if let Ok(o) = output {
        return !o.stdout.is_empty();
    }
    false
}

pub fn restart_ide() {
    println!("🚀 Restarting Antigravity...");
    let _ = std::process::Command::new("open")
        .arg("-a")
        .arg("Antigravity")
        .output();
}

pub fn apply_account_session(
    token_blob: &str,
    status_blob: &str,
    user_info_json: &serde_json::Value,
    user_plan_json: &serde_json::Value,
    credit_usage_json: &serde_json::Value,
) -> bool {
    let db_path = antigravity_db_path();
    let password = crypto::get_macos_safe_storage_password();
    let key = password.as_ref().map(|p| crypto::derive_key(p));

    match rusqlite::Connection::open(&db_path) {
        Ok(conn) => {
            let mut success = true;
            let _ = conn.busy_timeout(std::time::Duration::from_secs(2));

            // Ensure table exists (for clean IDE instances)
            let _ = conn.execute("CREATE TABLE IF NOT EXISTS ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)", []);

            // 1. Inject core tokens and status
            if let Err(e) = conn.execute(
                "INSERT OR REPLACE INTO ItemTable (key, value) VALUES ('antigravityUnifiedStateSync.oauthToken', ?1)",
                [token_blob],
            ) {
                eprintln!("DB Error (oauthToken): {}", e);
                success = false;
            }

            if let Err(e) = conn.execute(
                "INSERT OR REPLACE INTO ItemTable (key, value) VALUES ('antigravityUnifiedStateSync.userStatus', ?1)",
                [status_blob],
            ) {
                eprintln!("DB Error (userStatus): {}", e);
                success = false;
            }

            // Inject flags to bypass onboarding and bind machine identity
            let _ = conn.execute(
                "INSERT OR REPLACE INTO ItemTable (key, value) VALUES ('antigravityOnboarding', 'true')",
                [],
            );
            let existing_machine_id: Option<String> = conn
                .query_row(
                    "SELECT value FROM ItemTable WHERE key = 'storage.serviceMachineId'",
                    [],
                    |row| row.get(0),
                )
                .ok();
            if existing_machine_id.is_none() {
                let machine_id = uuid::Uuid::new_v4().to_string();
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO ItemTable (key, value) VALUES ('storage.serviceMachineId', ?1)",
                    [&machine_id],
                );
            }

            // 2. Inject encrypted secrets
            if let Some(k) = key {
                let secrets = [
                    ("secret://aicoding.auth.userInfo", user_info_json),
                    ("secret://aicoding.auth.userPlan", user_plan_json),
                    ("secret://aicoding.auth.creditUsage", credit_usage_json),
                ];
                for (s_key, s_val) in secrets {
                    if let Ok(plaintext) = serde_json::to_vec(s_val) {
                        if let Ok(encrypted) = crypto::encrypt_v10(&plaintext, &k) {
                            let buffer_str = build_vscode_secret_buffer(&encrypted);
                            let _ = conn.execute(
                                "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?1, ?2)",
                                [s_key, &buffer_str],
                            );
                        }
                    }
                }
            }

            // 3. Cleanup conflicting keys
            let _ = conn.execute("DELETE FROM ItemTable WHERE key IN ('antigravity.profileUrl', 'antigravity.account')", []);

            // 4. Force checkpoint and release connection
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            drop(conn);

            success
        }
        Err(e) => {
            eprintln!("❌ Failed to open database connection: {}", e);
            false
        }
    }
}

pub fn get_current_ide_email() -> Option<String> {
    let db_path = antigravity_db_path();
    let temp_db = std::env::temp_dir().join("aag_id_hot.db");
    if fs::copy(&db_path, &temp_db).is_err() {
        return None;
    }
    let conn = rusqlite::Connection::open(&temp_db).ok()?;
    let mut stmt = conn
        .prepare("SELECT value FROM itemTable WHERE key = 'antigravityUnifiedStateSync.userStatus'")
        .ok()?;
    let mut rows = stmt.query([]).ok()?;

    while let Some(row) = rows.next().ok().flatten() {
        let val: String = row.get(0).ok()?;
        if let Some(payload) = decode_unified_state_payload(&val) {
            if let Some(email) = decode_email_from_user_status_payload(&payload) {
                let _ = fs::remove_file(&temp_db);
                return Some(email);
            }
        }
    }
    let _ = fs::remove_file(&temp_db);
    None
}
