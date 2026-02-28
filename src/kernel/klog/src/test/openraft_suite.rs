use super::common::{TestMemoryStoreBuilder, TestSqliteStoreBuilder, init_test_logging};

#[test]
pub fn test_mem_store() -> anyhow::Result<()> {
    init_test_logging();
    openraft::testing::Suite::test_all(TestMemoryStoreBuilder::new()).unwrap();
    Ok(())
}

#[test]
pub fn test_sqlite_store() -> anyhow::Result<()> {
    init_test_logging();
    openraft::testing::Suite::test_all(TestSqliteStoreBuilder::new()).unwrap();
    Ok(())
}
