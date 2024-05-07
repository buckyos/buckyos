use crate::{DNSConfig, DNSProvider, ETCDConfig, ETCDProvider, LocalProvider, NameQuery, NSCmdRegister, NSCmdRegisterRef, NSConfig, NSProvider, NSResult, ProviderType};

pub struct NSApp {
    cmd_register: NSCmdRegisterRef,
}

impl NSApp {
    pub fn new() -> Self {
        Self {
            cmd_register: NSCmdRegisterRef::new(NSCmdRegister::new()),
        }
    }

    pub async fn load_from_config(&self, config: NSConfig) -> NSResult<NameQuery> {
        let mut query = NameQuery::new();
        if config.local_info_path.is_some() {
            let provider = Box::new(LocalProvider::new(config.local_info_path.clone().unwrap()));
            provider.load(self.cmd_register.as_ref()).await?;
            query.add_provider(provider);
        }
        for provider in config.provide_list {
            let provider = match provider.ty {
                ProviderType::DNS => {
                    let config = provider.get::<DNSConfig>()?;
                    Box::new(DNSProvider::new(config.dns_server)) as Box<dyn NSProvider>
                }
                ProviderType::ETCD => {
                    let config = provider.get::<ETCDConfig>()?;
                    Box::new(ETCDProvider::new(config.etcd_url)) as Box<dyn NSProvider>
                }
            };
            provider.load(self.cmd_register.as_ref()).await?;
            query.add_provider(provider);
        }

        if config.default_info_path.is_some() {
            let provider = Box::new(LocalProvider::new(config.default_info_path.clone().unwrap()));
            provider.load(self.cmd_register.as_ref()).await?;
            query.add_provider(provider);
        }
        Ok(query)
    }
}
