use serde::Deserialize;

#[cfg(target_os = "windows")]
use windows_sys::Win32::Security::Credentials::{
    CredReadW, CredWriteW, CREDENTIALW, CRED_TYPE_GENERIC, CRED_PERSIST_LOCAL_MACHINE,
};
#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;

#[derive(Deserialize, serde::Serialize)]
struct KeyringTokenDetails {
    access_token: String,
    token_type: String,
    refresh_token: String,
    expiry: String,
}

#[derive(Deserialize, serde::Serialize)]
struct KeyringPayload {
    token: KeyringTokenDetails,
    auth_method: String,
}

const CLIENT_ID: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "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";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

#[cfg(target_os = "windows")]
fn read_generic_credential(target_name: &str) -> Option<Vec<u8>> {
    let target_wide: Vec<u16> = std::ffi::OsStr::new(target_name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mut cred_ptr: *mut CREDENTIALW = std::ptr::null_mut();
        let res = CredReadW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred_ptr);
        
        if res == 0 || cred_ptr.is_null() {
            return None;
        }

        let cred = &*cred_ptr;
        let blob_slice = std::slice::from_raw_parts(
            cred.CredentialBlob as *const u8,
            cred.CredentialBlobSize as usize,
        );
        let bytes = blob_slice.to_vec();
        
        // Free the credential structure
        windows_sys::Win32::Security::Credentials::CredFree(cred_ptr as *mut std::ffi::c_void);
        
        Some(bytes)
    }
}

#[cfg(target_os = "windows")]
pub fn write_credential(target_name: &str, secret: &[u8]) -> bool {
    let target_wide: Vec<u16> = std::ffi::OsStr::new(target_name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
        
    unsafe {
        let mut cred: CREDENTIALW = std::mem::zeroed();
        cred.Type = CRED_TYPE_GENERIC;
        cred.TargetName = target_wide.as_ptr() as *mut _;
        cred.CredentialBlobSize = secret.len() as u32;
        cred.CredentialBlob = secret.as_ptr() as *mut _;
        cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
        
        let res = CredWriteW(&cred, 0);
        res != 0
    }
}

#[cfg(not(target_os = "windows"))]
fn read_generic_credential(_target_name: &str) -> Option<Vec<u8>> {
    // Only implemented for Windows in this codebase
    None
}

#[cfg(not(target_os = "windows"))]
pub fn write_credential(_target_name: &str, _secret: &[u8]) -> bool {
    false
}

/// Tries to fetch the Gemini Code Assist access token from the local Antigravity-Manager's Windows Credential Manager entry.
/// Automatically refreshes it if it is expired or expiring soon.
pub async fn get_or_refresh_credential(client: &reqwest::Client, target_name: &str, client_id: &str, client_secret: Option<&str>) -> Option<String> {
    tracing::info!("get_or_refresh_antigravity_token: starting");
    
    let password_bytes = match read_generic_credential(target_name) {
        Some(b) => b,
        None => {
            tracing::error!("get_or_refresh_antigravity_token: failed to read gemini:antigravity credential");
            return None;
        }
    };
    
    // The password from the Windows Credential manager might be raw JSON bytes.
    // Ensure we parse it as UTF-8 and trim null bytes
    let clean_json = String::from_utf8_lossy(&password_bytes)
        .trim_matches(char::from(0))
        .to_string();
    tracing::info!("get_or_refresh_antigravity_token: clean_json length: {}", clean_json.len());
    
    let mut payload = match serde_json::from_str::<KeyringPayload>(&clean_json) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("get_or_refresh_antigravity_token: failed to parse JSON: {}", e);
            return None;
        }
    };

    // Check expiry
    let is_expired = if let Ok(expiry) = chrono::DateTime::parse_from_rfc3339(&payload.token.expiry) {
        let now = chrono::Utc::now();
        // If it expires in less than 5 minutes (300 seconds), refresh it
        now.timestamp() + 300 >= expiry.timestamp()
    } else {
        true // If we can't parse expiry, assume it's expired to be safe
    };

    if is_expired {
        tracing::info!("gemini:antigravity token in keyring is expired or expiring soon, refreshing...");
        
        let req = client.post(TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded");
            
        let mut body_str = format!(
            "grant_type=refresh_token&refresh_token={}&client_id={}",
            payload.token.refresh_token, client_id
        );
        if let Some(secret) = client_secret {
            body_str.push_str(&format!("&client_secret={}", secret));
        }
        
        if let Ok(res) = req.body(body_str).send().await {
            if let Ok(res_json) = res.json::<serde_json::Value>().await {
                if let Some(access_token) = res_json.get("access_token").and_then(|v| v.as_str()) {
                    payload.token.access_token = access_token.to_string();
                    
                    if let Some(expires_in) = res_json.get("expires_in").and_then(|v| v.as_i64()) {
                        let new_expiry = chrono::Utc::now() + chrono::Duration::seconds(expires_in);
                        payload.token.expiry = new_expiry.to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
                    }
                    
                    if let Some(new_refresh) = res_json.get("refresh_token").and_then(|v| v.as_str()) {
                        payload.token.refresh_token = new_refresh.to_string();
                    }
                    
                    // Write back to keyring
                    if let Ok(new_json) = serde_json::to_string(&payload) {
                        if write_credential(target_name, new_json.as_bytes()) {
                            tracing::info!("Successfully refreshed gemini:antigravity token and updated Windows Credential Manager");
                        } else {
                            tracing::error!("Failed to write refreshed token to Windows Credential Manager");
                        }
                    }
                }
            }
        }
    }
    
    Some(payload.token.access_token)
}
