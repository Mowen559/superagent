use hub_rs::services::subscription;
#[tokio::main]
async fn main() {
    subscription::handle_quota().await;
}