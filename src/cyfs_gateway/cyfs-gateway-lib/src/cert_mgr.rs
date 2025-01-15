use rand::Rng;
use tokio::fs;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use rustls::server::{ResolvesServerCert, ClientHello};
use rustls::sign::CertifiedKey;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::task;
use log::*;
use crate::acme_client::{AcmeClient, AcmeOrderSession, AcmeChallengeResponder, AcmeAccount};
use crate::config::TlsConfig;

#[derive(Clone)]
struct CertInfo {
    key: Arc<CertifiedKey>, 
    expires: chrono::DateTime<chrono::Utc>,
}

enum CertState {
    None, 
    Ready(CertInfo),
    Expired(CertInfo),
}

struct CertMutPart<R: AcmeChallengeResponder> {
    state: CertState,
    ordering: bool,
    order: Option<AcmeOrderSession<R>>,
}

struct CertStubInner<R: AcmeChallengeEntry> {
    domains: Vec<String>,
    keystore_path: String,
    acme_client: AcmeClient,
    responder: Arc<R>,
    config: TlsConfig,
    mut_part: Mutex<CertMutPart<R::Responder>>,
}

struct CertStub<R: AcmeChallengeEntry> {
    inner: Arc<CertStubInner<R>>
}

impl<R: AcmeChallengeEntry> Clone for CertStub<R> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}


impl<R: AcmeChallengeEntry> std::fmt::Display for CertStub<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CertStub domains: {}", self.inner.domains.join(","))
    }
}


impl<R: AcmeChallengeEntry> CertStub<R> {
    fn new(domains: Vec<String>, keystore_path: String, acme_client: AcmeClient, responder: Arc<R>, config: TlsConfig) -> Self {
        Self {
            inner: Arc::new(CertStubInner {
                domains,
                keystore_path,
                acme_client,
                responder,
                config,
                mut_part: Mutex::new(CertMutPart {
                    state: CertState::None,
                    ordering: false,
                    order: None,
                }),
            })
        }
    }

    fn create_certified_key(cert_data: &[u8], key_data: &[u8]) -> Result<CertifiedKey> {
        let cert_chain = vec![rustls_pemfile::certs(&mut &*cert_data)?.remove(0)];
        let key = rustls::PrivateKey(rustls_pemfile::pkcs8_private_keys(&mut &*key_data)?.remove(0));
        
        let signing_key = rustls::sign::any_supported_type(&key)
            .map_err(|e| anyhow::anyhow!("Invalid private key: {}", e))?;
        
        let cert_chain = cert_chain.into_iter().map(rustls::Certificate).collect();
        Ok(CertifiedKey::new(cert_chain, signing_key))
    }

    pub fn get_cert(&self) -> Option<Arc<CertifiedKey>> {
        let mut_part = self.inner.mut_part.lock().unwrap();
        if let CertState::Ready(info) = &mut_part.state {
            Some(info.key.clone())
        } else {
            None
        }
    }

    pub async fn load_cert(&self) -> Result<()> {
        let cert_path = if let Some(path) = &self.inner.config.cert_path {
            path.clone()
        } else {
            // 尝试从 keystore_path 加载最新的证书
            let dir = tokio::fs::read_dir(&self.inner.keystore_path).await
                .map_err(|e| anyhow::anyhow!("read keystore dir failed, stub: {}, path: {}, {}", self, self.inner.keystore_path, e))?;
            
            let mut entries = Vec::new();
            tokio::pin!(dir);
            while let Some(entry) = dir.next_entry().await? {
                if entry.file_name().to_string_lossy().ends_with(".cert") {
                    entries.push(entry.path());
                }
            }
            
            if entries.is_empty() {
                // 如果没有找到证书，启动证书申请流程
                info!("no cert found in keystore, start ordering new cert, stub: {}", self);
                self.start_order().await?;
                return Ok(());
            }
            
            // 按文件名（时间戳）排序，取最新的
            entries.sort_by(|a, b| b.file_name().unwrap().cmp(a.file_name().unwrap()));
            entries[0].to_string_lossy().to_string()
        };

        info!("load cert, stub: {}, cert_path: {}", self, cert_path);
        let key_path = if self.inner.config.key_path.is_some() {
            self.inner.config.key_path.as_ref().unwrap().clone()
        } else {
            // 将 .cert 替换为 .key
            cert_path.replace(".cert", ".key")
        };

        let cert_data = fs::read(&cert_path).await
            .map_err(|e| anyhow::anyhow!("load cert failed, stub: {}, cert_path: {}, {}", self, cert_path, e))?;
        let key_data = fs::read(&key_path).await
            .map_err(|e| anyhow::anyhow!("load cert failed, stub: {}, key_path: {}, {}", self, key_path, e))?;
        
        let certified_key = Self::create_certified_key(&cert_data, &key_data)?;
        info!("load cert success, stub: {}, cert_path: {}, key_path: {}", self, cert_path, key_path);

        let mut mut_part = self.inner.mut_part.lock().unwrap();
        mut_part.state = CertState::Ready(CertInfo {
            key: Arc::new(certified_key),
            expires: chrono::Utc::now() + chrono::Duration::days(90)
        });

        Ok(())
    }

    pub async fn check_cert(&self) -> Result<()> {
        let should_order = {
            let mut mut_part = self.inner.mut_part.lock().unwrap();
            if mut_part.ordering {
                return Ok(());
            }
            
            match &mut_part.state {
                CertState::None => true,
                CertState::Ready(info) => {
                    let thirty_days_later = chrono::Utc::now() + chrono::Duration::days(30);
                    if info.expires <= thirty_days_later {
                        mut_part.state = CertState::Expired(info.clone());
                        true
                    } else {
                        false
                    }
                }
                CertState::Expired(_) => true
            }
        };

        if should_order {
            self.start_order().await?;
        }

        Ok(())
    }

    async fn order_inner(&self) -> Result<CertifiedKey> {
        let order = AcmeOrderSession::new(
            self.inner.domains.clone(),
            self.inner.acme_client.clone(),
            self.inner.responder.create_challenge_responder()
        );
        let (cert_data, key_data) = order.start().await?;
        
        let timestamp = chrono::Utc::now().timestamp();
        let cert_path = format!("{}/{}.cert", self.inner.keystore_path, timestamp);
        let key_path = format!("{}/{}.key", self.inner.keystore_path, timestamp);

        fs::write(&cert_path, &cert_data).await?;
        fs::write(&key_path, &key_data).await?;

        info!("save cert success, stub: {}, cert_path: {}, key_path: {}", self, cert_path, key_path);
        Self::create_certified_key(&cert_data, &key_data)
    }

    async fn start_order(&self) -> Result<()> {
        {
            let mut mut_part = self.inner.mut_part.lock().unwrap();
            if mut_part.ordering {
                return Ok(());
            }
            mut_part.ordering = true;
        }
      
        let result = self.order_inner().await;
        
        let mut mut_part = self.inner.mut_part.lock().unwrap();
        mut_part.ordering = false;
        match result {
            Ok(certified_key) => {
                mut_part.state = CertState::Ready(CertInfo {
                    key: Arc::new(certified_key),
                    expires: chrono::Utc::now() + chrono::Duration::days(90)
                });
                Ok(())
            }
            Err(e) => {
                Err(e)
            }
        }
    }
}

pub trait AcmeChallengeEntry: Send + Sync {
    type Responder: AcmeChallengeResponder;
    fn create_challenge_responder(&self) -> Self::Responder;
}

pub struct CertManager<R: AcmeChallengeEntry> {
    keystore_path: String,
    acme_client: AcmeClient,
    responder: Arc<R>,
    certs: HashMap<String, CertStub<R>>,
}

impl<R: 'static + AcmeChallengeEntry> std::fmt::Display for CertManager<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CertManager")
    }
}

impl<R: 'static + AcmeChallengeEntry> CertManager<R> {
    pub async fn new(keystore_path: String, responder: R) -> Result<Self> {
        info!("create cert manager, keystore_path: {}", keystore_path);
        // 检查是否存在 acme_account.json
        let account_path = format!("{}/acme_account.json", &keystore_path);
        let account = match AcmeAccount::from_file(Path::new(&account_path)).await {
            Ok(account) => {
                info!("从{}加载ACME账号", account_path);
                account
            }
            Err(_) => {
                // 生成随机邮箱并创建新账号
                let random_str: String = rand::thread_rng()
                    .sample_iter(&rand::distributions::Alphanumeric)
                    .take(10)
                    .map(char::from)
                    .collect();
                let email = format!("{}@buckyos.com", random_str);
                info!("生成随机邮箱地址: {}", email);
                
                AcmeAccount::new(email)
            }
        };

        let acme_client = AcmeClient::new(account).await?;
        let account = acme_client.account();
        if let Err(e) = account.save_to_file(Path::new(&account_path)).await {
            error!("保存ACME账号失败: {}", e);
        }
        Ok(Self {
            keystore_path,
            acme_client,
            responder: Arc::new(responder),
            certs: HashMap::new(),
        })
    }

    pub fn insert_config(&mut self, host: String, config: TlsConfig) -> Result<()> {
        info!("insert cert config, manager: {}, host: {}, config: {:?}", self, host, config);
        // 为域名列表生成唯一的keystore路径
        let keystore_path = format!("{}/{}", self.keystore_path, host);

        // 确保目录存在
        if let Err(e) = std::fs::create_dir_all(&keystore_path) {
            error!("创建证书存储目录失败: {}", e);
            return Err(anyhow::anyhow!("创建证书存储目录失败: {}", e));
        }
        let cert_stub = CertStub::new(
            vec![host.clone()], 
            keystore_path, 
            self.acme_client.clone(), 
            self.responder.clone(), 
            config);
        self.certs.insert(host, cert_stub.clone());
        
        task::spawn(async move {
            let _ = cert_stub.load_cert().await;
        });
        Ok(())
    }

    fn get_cert_by_host(&self,host:&str) -> Option<CertStub<R>> {
        let cert = self.certs.get(host);
        if cert.is_some() {
            info!("find tls config for host: {}", host);
            return Some(cert.unwrap().clone());
        }

        for (key,value) in self.certs.iter() {
            if key.starts_with("*.") {
                if host.ends_with(&key[2..]) {
                    info!("find tls config for host: {} ==> key:{}",host,key);
                    return Some(value.clone());
                }
            }
        }

        None
    }
}

impl<R: AcmeChallengeEntry> std::fmt::Debug for CertManager<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CertManager")
    }
}

impl<R: 'static + AcmeChallengeEntry> ResolvesServerCert for CertManager<R> {
    fn resolve(&self, client_hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        let server_name = client_hello.server_name().unwrap_or("").to_string();
        let cert_stub = self.get_cert_by_host(&server_name);
        if cert_stub.is_some() {
            return cert_stub.unwrap().get_cert();
        }
        None
    }
}
