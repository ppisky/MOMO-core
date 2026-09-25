#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    momo_server::run().await
}
