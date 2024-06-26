#[async_trait::async_trait]
pub trait Transaction {
    async fn begin_transaction(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn commit_transaction(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
