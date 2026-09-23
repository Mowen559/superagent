use crate::config::llm_settings::{self, LLMAccount};
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use reqwest::Client;
use serde::Deserialize;
use std::net::SocketAddr;
use tokio::sync::oneshot;
use uuid::Uuid;


use super::flow::TokenRes;

#[derive(Debug, Deserialize)]
pub(crate) struct CallbackQuery {
    pub(crate) code: Option<String>,
    pub(crate) state: Option<String>,
    pub(crate) error: Option<String>,
}

pub(crate) struct AppState {
    pub(crate) expected_state: String,
    pub(crate) tx: std::sync::Mutex<Option<oneshot::Sender<Result<String, String>>>>,
}

pub(crate) async fn execute_pkce_flow(
    client_id: &str,
    client_secret: Option<&str>,
    auth_endpoint: &str,
    token_endpoint: &str,
    scope: &str,
    port: u16,
    redirect_uri: &str,
    redirect_path: &str,
    extra_auth_params: &str,
) -> Result<(String, Option<String>, Option<i64>), String> {
    let (verifier, challenge, state) = crate::shared_kernel::crypto::generate_pkce_params();

    // Construct Authorization URL
    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&scope={}&state={}{}",
        auth_endpoint,
        client_id,
        urlencoding::encode(redirect_uri),
        challenge,
        urlencoding::encode(scope),
        state,
        extra_auth_params
    );

    // Start Local Server to receive callback
    let (tx, rx) = oneshot::channel();
    let app_state = std::sync::Arc::new(AppState {
        expected_state: state.clone(),
        tx: std::sync::Mutex::new(Some(tx)),
    });

    let app = Router::new()
        .route(redirect_path, get(handle_callback))
        .with_state(app_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("Failed to bind to port {}: {}", port, e))?;

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    // Spawn server task
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });

    // Open Browser
    println!("Opening your browser to log in...");
    println!("{}", auth_url);
    if let Err(e) = open::that(&auth_url) {
        eprintln!(
            "Failed to automatically open browser ({}). Please open the URL manually.",
            e
        );
    }

    // Wait for Callback
    let code = match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
        Err(_) => {
            let _ = shutdown_tx.send(());
            return Err("Login timed out after 5 minutes".to_string());
        }
        Ok(res) => match res {
            Ok(Ok(c)) => {
                let _ = shutdown_tx.send(());
                c
            }
            Ok(Err(e)) => {
                let _ = shutdown_tx.send(());
                return Err(format!("Login failed: {}", e));
            }
            Err(_) => {
                let _ = shutdown_tx.send(());
                return Err("Server channel closed unexpectedly".to_string());
            }
        }
    };

    println!("[Auth] Received callback. Exchanging code for token...");

    // Exchange Code for Token
    let client = Client::new();
    let req = client
        .post(token_endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded");

    let mut body = format!(
        "grant_type=authorization_code&code={}&client_id={}&code_verifier={}&redirect_uri={}",
        code,
        client_id,
        verifier,
        urlencoding::encode(redirect_uri)
    );

    if let Some(secret) = client_secret {
        body.push_str(&format!("&client_secret={}", secret));
    }

    let token_res_text = req
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Token request failed: {}", e))?
        .text()
        .await
        .map_err(|e| format!("Failed to read token response: {}", e))?;

    let token_res: TokenRes = serde_json::from_str(&token_res_text)
        .map_err(|e| format!("Failed to parse token response ({}): {}", e, token_res_text))?;

    if let Some(err) = token_res.error {
        return Err(format!("OAuth Error during token exchange: {}", err));
    }

    let access_token = token_res.access_token.ok_or("No access_token returned")?;
    Ok((access_token, token_res.refresh_token, token_res.expires_in))
}

pub(crate) async fn handle_callback(
    State(state): State<std::sync::Arc<AppState>>,
    Query(query): Query<CallbackQuery>,
) -> impl IntoResponse {
    let mut tx_guard = state.tx.lock().unwrap();
    if let Some(tx) = tx_guard.take() {
        if let Some(err) = query.error {
            let _ = tx.send(Err(err));
            return Html("<html><body><h1>Login Failed</h1><p>Check the terminal.</p></body><script>window.close()</script></html>");
        }

        let req_state = query.state.unwrap_or_default();
        if req_state != state.expected_state {
            let _ = tx.send(Err("State mismatch. Possible CSRF attack.".into()));
            return Html("<html><body><h1>Login Failed</h1><p>State mismatch.</p></body><script>window.close()</script></html>");
        }

        if let Some(code) = query.code {
            let _ = tx.send(Ok(code));
            return Html("<html><body><h1>Login Successful!</h1><p>You can close this tab and return to the terminal.</p></body><script>window.close()</script></html>");
        }

        let _ = tx.send(Err("No code found in callback".into()));
        Html("<html><body><h1>Login Failed</h1><p>No code found.</p></body><script>window.close()</script></html>")
    } else {
        Html("<html><body><p>Login already processed. You can close this tab.</p></body><script>window.close()</script></html>")
    }
}

// -----------------------------------------------------------------------------
// DeepCloud Custom Auth Flow
// -----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct DeepcloudCallbackQuery {
    pub(crate) token: Option<String>,
    pub(crate) error: Option<String>,
}

pub async fn handle_deepcloud_flow() -> Result<(), String> {
    let port = 1455;
    let redirect_uri = format!("http://localhost:{}/auth/deepcloud/callback", port);

    // Default to a common local dev port if not specified,
    // deepcloud-ui usually runs on 3000
    let deepcloud_url = "http://localhost:3000/cli/login";

    let login_url = format!(
        "{}?callback={}",
        deepcloud_url,
        urlencoding::encode(&redirect_uri)
    );

    println!("Opening browser to login to DeepCloud: {}", login_url);
    if webbrowser::open(&login_url).is_err() {
        println!("Could not open browser automatically. Please navigate to:");
        println!("{}", login_url);
    }

    let (tx, rx) = oneshot::channel();
    let state = std::sync::Arc::new(AppState {
        expected_state: "".to_string(), // not strictly needed for this simplified flow
        tx: std::sync::Mutex::new(Some(tx)),
    });

    let app = Router::new()
        .route("/auth/deepcloud/callback", get(handle_deepcloud_callback))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| e.to_string())?;

    let server = axum::serve(listener, app);

    tokio::select! {
        _ = server => {
            Err("Server stopped unexpectedly".into())
        }
        res = rx => {
            let token = res.map_err(|_| "Failed to receive token".to_string())??;

            // Save to keyring
            let entry = keyring::Entry::new("deepcloud_hub", "access_token")
                .map_err(|e| format!("Failed to access keyring: {}", e))?;

            entry.set_password(&token)
                .map_err(|e| format!("Failed to save token to keyring: {}", e))?;

            println!("✅ Successfully logged into DeepCloud and saved credentials securely.");
            Ok(())
        }
    }
}

pub(crate) async fn handle_deepcloud_callback(
    Query(query): Query<DeepcloudCallbackQuery>,
    State(state): State<std::sync::Arc<AppState>>,
) -> impl IntoResponse {
    let mut tx_guard = state.tx.lock().unwrap();
    if let Some(tx) = tx_guard.take() {
        if let Some(err) = query.error {
            let _ = tx.send(Err(err));
            return Html("<html><body><h1>Login Failed</h1><p>Check the terminal.</p></body><script>window.close()</script></html>");
        }

        if let Some(token) = query.token {
            let _ = tx.send(Ok(token));
            return Html("<html><body><h1>Login Successful!</h1><p>You can close this tab and return to the terminal.</p></body><script>window.close()</script></html>");
        }

        let _ = tx.send(Err("No token found in callback".into()));
        Html("<html><body><h1>Login Failed</h1><p>No token found.</p></body><script>window.close()</script></html>")
    } else {
        Html("<html><body><p>Login already processed. You can close this tab.</p></body><script>window.close()</script></html>")
    }
}

