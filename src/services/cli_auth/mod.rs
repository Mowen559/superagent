use crate::config::llm_settings::{self, LLMAccount};
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use dialoguer::{theme::ColorfulTheme, Input, Select};
use rand::Rng;
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use tokio::sync::oneshot;
use uuid::Uuid;


pub mod flow;
pub use flow::*;
pub mod server;
pub use server::*;
pub mod ui;
pub use ui::*;

pub mod runtime;
pub use runtime::*;
