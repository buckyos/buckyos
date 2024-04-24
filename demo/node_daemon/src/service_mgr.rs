pub struct ServiceItem {
    name: String,
    version: String,
}

impl RunItemControl for ServiceItem {
    async fn deploy(&self) -> Result<(), String> {
        info!("deploy service item: {}-{}", self.name, self.version);
        Ok(())
    }

    async fn uninstall(&self) -> Result<(), String> {
        info!("uninstall service item: {}-{}", self.name, self.version);
        Ok(())
    }

    async fn start(&self) -> Result<(), String> {
        info!("start service item: {}-{}", self.name, self.version);
        Ok(())
    }

    async fn stop(&self) -> Result<(), String> {
        info!("stop service item: {}-{}", self.name, self.version);
        Ok(())
    }

    async fn update(&self) -> Result<(), String> {
        info!("update service item: {}-{}", self.name, self.version);
        Ok(())
    }
}

