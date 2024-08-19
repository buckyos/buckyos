use async_trait::async_trait;

#[async_trait]
pub trait KVStoreProvider: Send + Sync {
    async fn get(&self, key: String) -> Result<Option<String>, Box<dyn std::error::Error>>;
    async fn set(&self, key: String, value: String) -> Result<(), Box<dyn std::error::Error>>;
}
