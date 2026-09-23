//! CLI 认证与 OAuth / PKCE 登录流程

pub mod flow;
pub mod runtime;
pub mod server;
pub mod ui;

pub use flow::{
    clear_deepcloud_token, force_refresh_account_token, get_deepcloud_token,
    handle_claude_oauth_flow, handle_gemini_oauth_flow, handle_github_copilot_flow,
    handle_openai_pkce_flow, refresh_account_token_if_needed,
};
pub use runtime::AuthRuntime;
pub use server::handle_deepcloud_flow;
pub use ui::ensure_logged_in;
