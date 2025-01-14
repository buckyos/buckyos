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
use rustls_pemfile::{certs};
use std::io::BufReader;

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

    pub fn get_cert(&self) -> Option<Arc<CertifiedKey>> {
        let mut_part = self.inner.mut_part.lock().unwrap();
        if let CertState::Ready(info) = &mut_part.state {
            Some(info.key.clone())
        } else {
            None
        }
    }

    pub async fn load_cert(&self) -> Result<()> {
        if let Some(cert_path) = &self.inner.config.cert_path {
            let cert_data = fs::read(cert_path).await?;
            let key_data = fs::read(self.inner.config.key_path.as_ref().unwrap()).await?;
            
            let cert_chain = certs(&mut BufReader::new(&cert_data[..]))?
                .into_iter()
                .map(|der| rustls::Certificate(der))
                .collect();
            let key = rustls::PrivateKey(rustls_pemfile::pkcs8_private_keys(&mut &*key_data)?.remove(0));
            
            let signing_key = rustls::sign::any_supported_type(&key)
                .map_err(|e| anyhow::anyhow!("Invalid private key: {}", e))?;
            
            let certified_key = CertifiedKey::new(cert_chain, signing_key);
            let mut mut_part = self.inner.mut_part.lock().unwrap();
            mut_part.state = CertState::Ready(CertInfo {
                key: Arc::new(certified_key),
                expires: chrono::Utc::now() + chrono::Duration::days(90)
            });
        }
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

    async fn start_order(&self) -> Result<()> {
        let mut mut_part = self.inner.mut_part.lock().unwrap();
        if mut_part.ordering {
            return Ok(());
        }
        mut_part.ordering = true;

        let order = AcmeOrderSession::new(
            self.inner.domains.clone(),
            self.inner.acme_client.clone(),
            self.inner.responder.create_challenge_responder()
        );

        match order.start().await {
            Ok((cert_data, key_data)) => {
                // 保存证书文件
                let timestamp = chrono::Utc::now().timestamp();
                let cert_path = format!("{}/{}.cert", self.inner.keystore_path, timestamp);
                let key_path = format!("{}/{}.key", self.inner.keystore_path, timestamp);

                fs::write(&cert_path, &cert_data).await?;
                fs::write(&key_path, &key_data).await?;
                // 更新状态
                let cert_chain = vec![rustls_pemfile::certs(&mut &*cert_data)?.remove(0)];
                let key = rustls::PrivateKey(rustls_pemfile::pkcs8_private_keys(&mut &*key_data)?.remove(0));
                if let Ok(signing_key) = rustls::sign::any_supported_type(&key) {
                    let cert_chain = cert_chain.into_iter().map(rustls::Certificate).collect();
                    let certified_key = rustls::sign::CertifiedKey::new(cert_chain, signing_key);
                    mut_part.state = CertState::Ready(CertInfo {
                        key: Arc::new(certified_key),
                        expires: chrono::Utc::now() + chrono::Duration::days(90)
                    });
                }
            }
            Err(e) => {
                mut_part.ordering = false;
                return Err(e);
            }
        }

        mut_part.ordering = false;
        Ok(())
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

impl<R: 'static + AcmeChallengeEntry> CertManager<R> {
    pub async fn new(keystore_path: String, responder: R) -> Result<Self> {
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
                let email = format!("{}@example.com", random_str);
                info!("生成随机邮箱地址: {}", email);
                
                let account = AcmeAccount::new(email);
                if let Err(e) = account.save_to_file(Path::new(&account_path)).await {
                    error!("保存ACME账号失败: {}", e);
                }
                account
            }
        };

        let acme_client = AcmeClient::new(account).await.unwrap();
        Ok(Self {
            keystore_path,
            acme_client,
            responder: Arc::new(responder),
            certs: HashMap::new(),
        })
    }

    pub fn insert_config(&mut self, host: String, config: TlsConfig) -> Result<()> {
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
