//! acme_client.rs
//! ACME 客户端实现
use reqwest;
use serde::{Serialize, Deserialize};
use std::time::Duration;
use std::sync::Mutex;
use openssl::{
    pkey::{PKey, Private},
    rsa::Rsa,
};
use std::path::Path;
use tokio::fs;
use serde::de::DeserializeOwned;
use std::sync::Arc;
use base64::Engine;
use anyhow::Result;

/// ACME 目录结构
#[derive(Debug, Deserialize)]
struct Directory {
    #[serde(rename = "newNonce")]
    new_nonce: String,
    #[serde(rename = "newAccount")]
    new_account: String,
    #[serde(rename = "newOrder")]
    new_order: String,
    #[serde(rename = "revokeCert")]
    revoke_cert: String,
}

/// Nonce 管理器
#[derive(Debug)]
struct NonceManager {
    current_nonce: Mutex<Option<String>>,
}

impl NonceManager {
    fn new() -> Self {
        Self {
            current_nonce: Mutex::new(None),
        }
    }

    /// 获取 nonce,获取后当前 nonce 失效
    fn take_nonce(&self) -> Option<String> {
        let mut nonce = self.current_nonce.lock().unwrap();
        nonce.take()
    }

    /// 更新 nonce
    fn update_nonce(&self, new_nonce: String) {
        let mut nonce = self.current_nonce.lock().unwrap();
        *nonce = Some(new_nonce);
    }
}

/// ACME 账户信息
#[derive(Debug, Serialize, Deserialize)]
pub struct AcmeAccount {
    email: String,
    #[serde(serialize_with = "serialize_key", deserialize_with = "deserialize_key")]
    key: PKey<Private>,
}

impl AcmeAccount {
    pub fn new(email: String) -> Self {
        let rsa = Rsa::generate(2048).unwrap();
        let key = PKey::from_rsa(rsa).unwrap();
        
        Self { email, key }
    }

    pub async fn from_file(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path).await?;
        serde_json::from_str(&content).map_err(|e| e.into())
    }

    pub async fn save_to_file(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json).await?;
        Ok(())
    }
}

fn serialize_key<S>(key: &PKey<Private>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::Error;
    let pem = key.private_key_to_pem_pkcs8()
        .map_err(Error::custom)?;
    let pem_str = String::from_utf8(pem)
        .map_err(Error::custom)?;
    serializer.serialize_str(&pem_str)
}

fn deserialize_key<'de, D>(deserializer: D) -> Result<PKey<Private>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let pem_str = String::deserialize(deserializer)?;
    PKey::private_key_from_pem(pem_str.as_bytes())
        .map_err(Error::custom)
}

#[derive(Debug, Deserialize)]
struct AcmeError {
    type_: String,
    detail: String,
    status: u16,
}

#[derive(Debug, Deserialize)]
struct AcmeResponse<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<T>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    errors: Vec<AcmeError>,
}

/// ACME 客户端结构
#[derive(Debug, Clone)]
pub struct AcmeClient {
    inner: Arc<AcmeClientInner>,
}

#[derive(Debug)]
struct AcmeClientInner {
    directory: Directory,
    http_client: reqwest::Client,
    nonce_manager: NonceManager,
    account: AcmeAccount,
}

/// 挑战响应接口
#[async_trait::async_trait]
pub trait AcmeChallengeResponder: Send + Sync {
    /// 响应 HTTP 挑战
    async fn respond_http(&self, token: &str, key_auth: &str) -> Result<()>;
    
    /// 响应 DNS 挑战
    async fn respond_dns(&self, domain: &str, digest: &str) -> Result<()>;
    
    /// 响应 TLS-ALPN 挑战
    async fn respond_tls_alpn(&self, domain: &str, key_auth: &str) -> Result<()>;
    
    /// 清理挑战响应
    async fn cleanup(&self) -> Result<()>;
}

/// 证书订单会话
#[derive(Debug)]
pub struct AcmeOrderSession<R: AcmeChallengeResponder> {
    domains: Vec<String>,
    valid_days: u32,
    key_type: KeyType,
    status: OrderStatus,
    client: AcmeClient,
    responder: R,
    order_info: Option<OrderInfo>,
}

impl<R: AcmeChallengeResponder> AcmeOrderSession<R> {
    pub fn new(
        domains: Vec<String>,
        client: AcmeClient,
        responder: R
    ) -> Self {
        Self {
            domains,
            valid_days: 90,
            key_type: KeyType::Rsa2048,
            status: OrderStatus::New,
            client,
            responder,
            order_info: None,
        }
    }

    /// 开始证书申请流程
    pub async fn start(mut self) -> Result<(Vec<u8>, Vec<u8>)> {
        // 1. 创建订单
        let (authorizations, finalize_url) = self.client.create_order(&self.domains).await?;
        
        // 更新订单信息和状态
        self.order_info = Some(OrderInfo {
            authorizations,
            finalize_url,
        });
        self.update_status(OrderStatus::Pending);
        
        // 2. 处理每个授权
        if let Some(order_info) = &self.order_info {
            for auth_url in &order_info.authorizations {
                // 获取挑战信息
                let challenge = self.client.get_challenge(auth_url).await?;
                
                // 准备挑战响应
                match challenge.type_.as_str() {
                    "http-01" => {
                        self.responder.respond_http(&challenge.token, &challenge.key_auth).await?;
                    }
                    "dns-01" => {
                        self.responder.respond_dns(&challenge.domain, &challenge.digest).await?;
                    }
                    "tls-alpn-01" => {
                        self.responder.respond_tls_alpn(&challenge.domain, &challenge.key_auth).await?;
                    }
                    _ => return Err(anyhow::anyhow!("Unsupported challenge type")),
                }

                // 通知服务器验证挑战
                self.client.verify_challenge(&challenge.url).await?;
                
                // 等待验证完成
                self.client.poll_authorization(auth_url).await?;
            }
        }

        // 3. 完成订单
        if let Some(order_info) = &self.order_info {
            // 生成CSR
            let (csr, private_key) = self.client.generate_csr(&self.domains)?;
            
            // Finalize订单
            let cert_url = self.client.finalize_order(&order_info.finalize_url, &csr).await?;
            
            // 下载证书
            let cert = self.client.download_certificate(&cert_url).await?;
            
            // 清理挑战响应
            self.responder.cleanup().await?;
            
            Ok((cert, private_key))
        } else {
            Err(anyhow::anyhow!("No order information available"))
        }
    }
}

#[derive(Debug)]
struct OrderInfo {
    authorizations: Vec<String>,
    finalize_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OrderRequest {
    identifiers: Vec<Identifier>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Identifier {
    #[serde(rename = "type")]
    type_: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct OrderResponse {
    status: String,
    expires: String,
    identifiers: Vec<Identifier>,
    authorizations: Vec<String>,
    finalize: String,
}

impl<R: AcmeChallengeResponder> AcmeOrderSession<R> {
    // 更新状态的方法
    fn update_status(&mut self, new_status: OrderStatus) {
        self.status = new_status;
    }
}

#[derive(Debug, Deserialize)]
struct Challenge {
    type_: String,
    url: String,
    token: String,
    domain: String,
    key_auth: String,
    digest: String,
}

impl AcmeClient {
    // 已有方法改为使用 inner
    pub async fn new(account: AcmeAccount) -> Result<Self> {
        let http_client = reqwest::Client::new();
        
        // 从 ACME 服务器获取目录
        let directory: Directory = http_client
            .get("https://acme-v02.api.letsencrypt.org/directory")
            .send()
            .await?
            .json()
            .await?;

        let inner = AcmeClientInner {
            directory,
            http_client,
            nonce_manager: NonceManager::new(),
            account,
        };
        
        Ok(Self { inner: Arc::new(inner) })
    }

    // 新增方法
    async fn get_challenge(&self, auth_url: &str) -> Result<Challenge> {
        self.signed_post(auth_url, &serde_json::json!({})).await
    }

    async fn verify_challenge(&self, challenge_url: &str) -> Result<()> {
        self.signed_post(challenge_url, &serde_json::json!({})).await
    }

    /// 轮询授权状态
    async fn poll_authorization(&self, auth_url: &str) -> Result<()> {
        let max_attempts = 10;
        let wait_seconds = 3;

        for _ in 0..max_attempts {
            let response: AuthzResponse = self.signed_post(auth_url, &serde_json::json!({})).await?;
            
            match response.status.as_str() {
                "valid" => return Ok(()),
                "pending" => {
                    tokio::time::sleep(Duration::from_secs(wait_seconds)).await;
                    continue;
                },
                "invalid" => return Err(anyhow::anyhow!("Authorization failed")),
                _ => return Err(anyhow::anyhow!("Unexpected authorization status: {}", response.status)),
            }
        }
        
        Err(anyhow::anyhow!("Authorization polling timeout"))
    }

    /// 完成订单
    async fn finalize_order(&self, url: &str, csr: &[u8]) -> Result<String> {
        let payload = serde_json::json!({
            "csr": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(csr)
        });

        let response: FinalizeResponse = self.signed_post(url, &payload).await?;
        
        match response.status.as_str() {
            "valid" => Ok(response.certificate.ok_or_else(|| anyhow::anyhow!("No certificate URL in response"))?),
            _ => Err(anyhow::anyhow!("Order finalization failed: {}", response.status)),
        }
    }

    /// 下载证书
    async fn download_certificate(&self, url: &str) -> Result<Vec<u8>> {
        let response: Vec<u8> = self.signed_post(url, &serde_json::json!({})).await?;
        Ok(response)
    }

    /// 创建新的订单
    pub async fn create_order(&self, domains: &[String]) -> Result<(Vec<String>, String)> {
        // 构造订单请求
        let request = OrderRequest {
            identifiers: domains.iter().map(|domain| Identifier {
                type_: "dns".to_string(),
                value: domain.clone(),
            }).collect(),
        };

        // 发送订单请求
        let response: OrderResponse = self.signed_post(
            &self.inner.directory.new_order,
            &request
        ).await?;

        // 返回授权URL列表和finalize URL
        Ok((response.authorizations, response.finalize))
    }

    fn generate_csr(&self, domains: &[String]) -> Result<(Vec<u8>, Vec<u8>)> {
        let rsa = Rsa::generate(2048)?;
        let pkey = PKey::from_rsa(rsa)?;
        
        let mut builder = openssl::x509::X509ReqBuilder::new()?;
        builder.set_pubkey(&pkey)?;
        
        let mut name_builder = openssl::x509::X509NameBuilder::new()?;
        name_builder.append_entry_by_text("CN", &domains[0])?;
        builder.set_subject_name(&name_builder.build())?;
        
        builder.sign(&pkey, openssl::hash::MessageDigest::sha256())?;
        
        Ok((builder.build().to_pem()?, pkey.private_key_to_pem_pkcs8()?))
    }

    /// 发送签名的POST请求并处理响应
    async fn signed_post<T, R>(&self, url: &str, payload: &T) -> Result<R>
    where
        T: Serialize,
        R: DeserializeOwned,
    {
        let nonce = self.get_nonce().await?;
        let jws = self.sign_request(url, &nonce, payload)?;
        
        let response = self.inner.http_client
            .post(url)
            .json(&jws)
            .send()
            .await?;

        self.handle_response(response).await
    }

    /// 获取 nonce
    async fn get_nonce(&self) -> Result<String> {
        if let Some(nonce) = self.inner.nonce_manager.take_nonce() {
            Ok(nonce)
        } else {
            self.fetch_new_nonce().await
        }
    }

    /// 从服务器获取新的 nonce
    async fn fetch_new_nonce(&self) -> Result<String> {
        let response = self.inner.http_client
            .head(&self.inner.directory.new_nonce)
            .send()
            .await?;

        let nonce = response
            .headers()
            .get("Replay-Nonce")
            .ok_or_else(|| anyhow::anyhow!("No nonce found"))?
            .to_str()?
            .to_string();

        self.inner.nonce_manager.update_nonce(nonce.clone());
        Ok(nonce)
    }

    /// 处理ACME响应
    async fn handle_response<R>(&self, response: reqwest::Response) -> Result<R>
    where
        R: DeserializeOwned,
    {
        // 更新nonce
        if let Some(new_nonce) = response.headers().get("Replay-Nonce") {
            if let Ok(nonce) = new_nonce.to_str() {
                self.inner.nonce_manager.update_nonce(nonce.to_string());
            }
        }

        // 检查状态码
        let status = response.status();
        if !status.is_success() {
            let error: AcmeResponse<()> = response.json().await?;
            if !error.errors.is_empty() {
                return Err(anyhow::anyhow!(
                    "ACME error: {} - {}", 
                    error.errors[0].type_,
                    error.errors[0].detail
                ));
            }
            return Err(anyhow::anyhow!("HTTP error: {}", status));
        }

        // 解析响应体
        let result = response.json().await?;
        Ok(result)
    }

    /// 签名请求数据
    fn sign_request<T: Serialize>(
        &self,
        url: &str,
        nonce: &str,
        payload: &T,
    ) -> Result<serde_json::Value> {
        let payload_str = serde_json::to_string(payload)?;
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_str);
        
        let protected = serde_json::json!({
            "alg": "RS256",
            "nonce": nonce,
            "url": url,
            "jwk": {
                "kty": "RSA",
                "n": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.inner.account.key.rsa()?.n().to_vec()),
                "e": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.inner.account.key.rsa()?.e().to_vec()),
            }
        });
        
        let protected_str = serde_json::to_string(&protected)?;
        let protected_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(protected_str);
        
        let signing_input = format!("{}.{}", protected_b64, payload_b64);
        let mut signer = openssl::sign::Signer::new(
            openssl::hash::MessageDigest::sha256(),
            &self.inner.account.key
        )?;
        let mut signature = vec![0; signer.len()?];
        signer.sign_oneshot(&mut signature, signing_input.as_bytes())?;
        
        let signature_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature);
        
        Ok(serde_json::json!({
            "protected": protected_b64,
            "payload": payload_b64,
            "signature": signature_b64,
        }))
    }
}


#[derive(Debug)]
pub enum KeyType {
    Rsa2048,
    Rsa4096,
    // 可以后续添加 ECC 等其他类型
}

#[derive(Debug, PartialEq)]
pub enum OrderStatus {
    New,
    Pending,
    Ready,
    Processing,
    Valid,
    Invalid,
}

#[derive(Debug, Deserialize)]
struct AuthzResponse {
    status: String,
    challenges: Vec<Challenge>,
}

#[derive(Debug, Deserialize)]
struct FinalizeResponse {
    status: String,
    certificate: Option<String>,
    #[serde(default)]
    error: Option<AcmeError>,
}