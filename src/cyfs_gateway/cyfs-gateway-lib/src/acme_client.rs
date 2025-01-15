//! acme_client.rs
//! ACME 客户端实现
use reqwest;
use serde::{Serialize, Deserialize};
use std::{sync::RwLock, time::Duration};
use std::sync::Mutex;
use openssl::{
    pkey::{PKey, Private},
    rsa::Rsa,
    x509::{X509ReqBuilder, X509NameBuilder, extension::SubjectAlternativeName},
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


struct AccountInner {
    email: String,
    key: PKey<Private>,
    kid: RwLock<Option<String>>,
}

/// ACME 账户信息
#[derive(Serialize, Deserialize)]
#[serde(into = "AccountConfig")]
#[serde(from = "AccountConfig")]
pub struct AcmeAccount {
    inner: Arc<AccountInner>,
}

impl Clone for AcmeAccount {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

impl Into<AccountConfig> for AcmeAccount {
    fn into(self) -> AccountConfig {
        let key_pem = self.inner.key.private_key_to_pem_pkcs8()
            .expect("Failed to encode private key to PEM");
        let key_str = String::from_utf8(key_pem)
            .expect("Failed to convert PEM to string");

        AccountConfig {
            email: self.inner.email.clone(),
            key: key_str,
            kid: self.inner.kid.read().unwrap().clone(),
        }
    }
}


impl From<AccountConfig> for AcmeAccount {
    fn from(inner: AccountConfig) -> Self {
        let key_pem = inner.key.as_bytes();
        let key = PKey::private_key_from_pem(key_pem)
            .expect("Failed to parse private key from PEM");
            
        Self {
            inner: Arc::new(AccountInner {
                email: inner.email,
                key,
                kid: RwLock::new(inner.kid),
            }),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct AccountConfig {
    email: String,
    key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    kid: Option<String>,
}


impl std::fmt::Display for AcmeAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AcmeAccount(email: {})", self.inner.email)
    }
}

impl AcmeAccount {
    pub fn new(email: String) -> Self {
        info!("generate acme account key for {}", email);
        let rsa = Rsa::generate(2048).unwrap();
        let key = PKey::from_rsa(rsa).unwrap();
        
        Self { inner: Arc::new(AccountInner { email, key, kid: RwLock::new(None) }) }
    }

    pub async fn from_file(path: &Path) -> Result<Self> {
        info!("load acme account key from {}", path.display());
        let content = fs::read_to_string(path).await
            .map_err(|e| {
                error!("read acme account key file {} failed, {}", path.display(), e);
                anyhow::anyhow!("read acme account key file {} failed, {}", path.display(), e)
            })?;
        
        let account = serde_json::from_str(&content).map_err(|e| {
            error!("parse acme account key file {} failed, {}", path.display(), e);
            anyhow::anyhow!("parse acme account key file {} failed, {}", path.display(), e)
        })?;

        info!("load acme account key from {} success, account: {}", path.display(), account);
        Ok(account)
    }

    pub async fn save_to_file(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json).await
            .map_err(|e| {
                error!("save acme account key to {} failed, {}", path.display(), e);
                anyhow::anyhow!("save acme account key to {} failed, {}", path.display(), e)
            })?;
        
        info!("save acme account key to {} success, account: {}", path.display(), self);
        Ok(())
    }

    pub fn email(&self) -> &str {
        &self.inner.email
    }

    pub fn key(&self) -> &PKey<Private> {
        &self.inner.key
    }

    pub fn kid(&self) -> Option<String> {
        self.inner.kid.read().unwrap().clone()
    }

    pub fn set_kid(&self, kid: String) {
        let mut kid_lock = self.inner.kid.write().unwrap();
        *kid_lock = Some(kid);
    }
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
#[derive(Clone)]
pub struct AcmeClient {
    inner: Arc<AcmeClientInner>,
}

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
    async fn respond_http(&self, domain: &str, token: &str, key_auth: &str) -> Result<()>;
    fn revert_http(&self, domain: &str, token: &str);
    
    /// 响应 DNS 挑战
    async fn respond_dns(&self, domain: &str, digest: &str) -> Result<()>;
    fn revert_dns(&self, domain: &str, digest: &str);

    /// 响应 TLS-ALPN 挑战
    async fn respond_tls_alpn(&self, domain: &str, key_auth: &str) -> Result<()>;
    fn revert_tls_alpn(&self, domain: &str, key_auth: &str);
}

/// 证书订单会话
pub struct AcmeOrderSession<R: AcmeChallengeResponder> {
    domains: Vec<String>,
    valid_days: u32,
    key_type: KeyType,
    status: OrderStatus,
    client: AcmeClient, 
    responder: R,
    respond_logs: Vec<Challenge>,
    order_info: Option<OrderInfo>,
}

impl<R: AcmeChallengeResponder> std::fmt::Display for AcmeOrderSession<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AcmeOrderSession(domains: {})", self.domains.join(","))
    }
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
            respond_logs: vec![],
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
                info!("got acme challenge, client: {}, challenge: {:?}", self, challenge);
                // 准备挑战响应
                match challenge.type_.as_str() {
                    "http-01" => {
                        self.responder.respond_http(challenge.domain.as_str(), &challenge.token, &challenge.key_auth).await?;
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

                self.respond_logs.push(challenge);
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
            
            Ok((cert, private_key))
        } else {
            Err(anyhow::anyhow!("No order information available"))
        }
    }
}


impl<R: AcmeChallengeResponder> Drop for AcmeOrderSession<R> {
    fn drop(&mut self) {
        for log in self.respond_logs.iter() {
            match log.type_.as_str() {
                "http-01" => {
                    self.responder.revert_http(log.domain.as_str(), log.token.as_str());
                }
                "dns-01" => {
                    self.responder.revert_dns(log.domain.as_str(), log.digest.as_str());
                }
                "tls-alpn-01" => {
                    self.responder.revert_tls_alpn(log.domain.as_str(), log.key_auth.as_str());
                },
                _ => {unreachable!()}
            }
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


impl std::fmt::Display for AcmeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AcmeClient(account: {})", self.inner.account)
    }
}

impl AcmeClient {
    // 已有方法改为使用 inner
    pub async fn new(account: AcmeAccount) -> Result<Self> {
        info!("create acme client, account: {}", account);
        let http_client = reqwest::Client::new();
        
        info!("get acme directory");
        // 从 ACME 服务器获取目录
        let directory: Directory = http_client
            .get("https://acme-v02.api.letsencrypt.org/directory")
            .send()
            .await
            .map_err(|e| {
                error!("get acme directory failed, {}", e);
                anyhow::anyhow!("get acme directory failed, {}", e)
            })?
            .json()
            .await
            .map_err(|e| {
                error!("parse acme directory failed, {}", e);
                anyhow::anyhow!("parse acme directory failed, {}", e)
            })?;
            
        info!("get acme directory success, directory: {:?}", directory);
        let inner = AcmeClientInner {
            directory,
            http_client,
            nonce_manager: NonceManager::new(),
            account,
        };
        
        let client = Self { inner: Arc::new(inner) };

        if client.account().kid().is_none() {
            client.register_account().await?;
        }
        Ok(client)
    }

    pub fn account(&self) -> &AcmeAccount {
        &self.inner.account
    }

    async fn register_account(&self) -> Result<()> {
        info!("register acme account, client: {}", self);
        let payload = serde_json::json!({
            "termsOfServiceAgreed": true,
            "contact": [
                format!("mailto:{}", self.account().email())
            ]
        });

        let nonce = self.get_nonce().await?;
        let jws = self.sign_request_new_account(&self.inner.directory.new_account, &nonce, &payload)?;
        
        let response = self.inner.http_client
            .post(&self.inner.directory.new_account)
            .header("Content-Type", "application/jose+json")
            .json(&jws)
            .send()
            .await
            .map_err(|e| {
                error!("register acme account failed, client: {}, {}", self, e);
                anyhow::anyhow!("register acme account failed, client: {}, {}", self, e)
            })?;

        if response.status().is_success() {
            // 获取账户 URL (kid)
            let kid = response.headers()
                .get("Location")
                .ok_or_else(|| anyhow::anyhow!("No Location header in new account response"))?
                .to_str()?
                .to_string();

            self.inner.account.set_kid(kid.clone());
            info!("got account kid: {}", kid);
        }
        
        let _: AccountResponse = self.handle_response(response).await?;
        Ok(())
    }

    /// 专门用于账户注册的签名请求
    fn sign_request_new_account<T: Serialize>(
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
            "jwk": {  // 对于新账户注册，使用 jwk 而不是 kid
                "kty": "RSA",
                "n": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.account().key().rsa()?.n().to_vec()),
                "e": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.account().key().rsa()?.e().to_vec()),
            }
        });
        
        let protected_str = serde_json::to_string(&protected)?;
        let protected_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(protected_str);
        
        let signing_input = format!("{}.{}", protected_b64, payload_b64);
        let mut signer = openssl::sign::Signer::new(
            openssl::hash::MessageDigest::sha256(),
            &self.account().key()
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

    // 新增方法
    async fn get_challenge(&self, auth_url: &str) -> Result<Challenge> {
        info!("get acme challenge, client: {}, auth_url: {}", self, auth_url);
        
        let response = self.inner.http_client
            .get(auth_url)
            .send()
            .await
            .map_err(|e| {
                error!("get acme challenge failed, client: {}, auth_url: {}, {}", self, auth_url, e);
                anyhow::anyhow!("get acme challenge failed, client: {}, auth_url: {}, {}", self, auth_url, e)
            })?;

        let authz: AuthzResponse = response.json().await?;
        
        // 选择 http-01 挑战
        let challenge = authz.challenges.iter()
            .find(|c| c.type_ == "http-01")
            .ok_or_else(|| anyhow::anyhow!("No http-01 challenge found"))?;

        // 计算 key authorization
        let key_auth = self.compute_key_authorization(&challenge.token)?;
        
        Ok(Challenge {
            type_: challenge.type_.clone(),
            url: challenge.url.clone(),
            token: challenge.token.clone(),
            domain: authz.identifier.value,
            key_auth,
            digest: "".to_string(), // 对于 http-01 挑战，不需要 digest
        })
    }

    fn compute_key_authorization(&self, token: &str) -> Result<String> {
        // 计算 key authorization: token + "." + base64url(JWK thumbprint)
        let jwk = serde_json::json!({
            "kty": "RSA",
            "n": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.account().key().rsa()?.n().to_vec()),
            "e": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.account().key().rsa()?.e().to_vec()),
        });

        let jwk_str = serde_json::to_string(&jwk)?;
        let mut hasher = openssl::sha::Sha256::new();
        hasher.update(jwk_str.as_bytes());
        let thumbprint = hasher.finish();
        
        Ok(format!("{}.{}", token, base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(thumbprint)))
    }

    /// 处理ACME响应，不关心返回值
    async fn handle_response_no_body(&self, response: reqwest::Response) -> Result<()> {
        // 更新nonce
        if let Some(new_nonce) = response.headers().get("Replay-Nonce") {
            if let Ok(nonce) = new_nonce.to_str() {
                self.inner.nonce_manager.update_nonce(nonce.to_string());
            }
        }

        // 检查状态码
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await?;
            error!("acme response error, status: {}, body: {}", status, body);
            return Err(anyhow::anyhow!("HTTP error: {} - {}", status, body));
        }

        Ok(())
    }

    async fn verify_challenge(&self, challenge_url: &str) -> Result<()> {
        info!("verify acme challenge, client: {}, challenge_url: {}", self, challenge_url);
        self.signed_post_no_body(challenge_url, &serde_json::json!({})).await
            .map_err(|e| {
                error!("verify acme challenge failed, client: {}, challenge_url: {}, {}", self, challenge_url, e);
                anyhow::anyhow!("verify acme challenge failed, client: {}, challenge_url: {}, {}", self, challenge_url, e)
            })
    }

    /// 轮询授权状态
    async fn poll_authorization(&self, auth_url: &str) -> Result<()> {
        info!("poll acme authorization, client: {}, auth_url: {}", self, auth_url);
        let max_attempts = 10;
        let wait_seconds = 3;

        for _ in 0..max_attempts {
            let response = self.inner.http_client
                .get(auth_url)
                .send()
                .await
                .map_err(|e| {
                    error!("poll acme authorization failed, client: {}, auth_url: {}, {}", self, auth_url, e);
                    anyhow::anyhow!("poll acme authorization failed, client: {}, auth_url: {}, {}", self, auth_url, e)
                })?;

            let authz: AuthzResponse = response.json().await?;
            
            match authz.status.as_str() {
                "valid" => {
                    info!("poll acme authorization success, client: {}, auth_url: {}", self, auth_url);
                    return Ok(());
                }
                "pending" => {
                    info!("poll acme authorization pending, client: {}, auth_url: {}, wait {} seconds", self, auth_url, wait_seconds);
                    tokio::time::sleep(Duration::from_secs(wait_seconds)).await;
                    continue;
                },
                "invalid" => {
                    error!("poll acme authorization failed, client: {}, auth_url: {}, status: {}", self, auth_url, authz.status);
                    return Err(anyhow::anyhow!("Authorization failed"));
                }
                _ => {
                    error!("poll acme authorization failed, client: {}, auth_url: {}, status: {}", self, auth_url, authz.status);
                    return Err(anyhow::anyhow!("Unexpected authorization status: {}", authz.status));
                }
            }
        }
        
        info!("poll acme authorization timeout, client: {}, auth_url: {}", self, auth_url);
        Err(anyhow::anyhow!("Authorization polling timeout"))
    }

    /// 完成订单
    async fn finalize_order(&self, url: &str, csr: &[u8]) -> Result<String> {
        info!("finalize acme order, client: {}, url: {}, csr: {}", self, url, csr.len());
        let payload = serde_json::json!({
            "csr": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(csr)
        });

        let response: FinalizeResponse = self.signed_post(url, &payload).await
            .map_err(|e| {
                error!("finalize acme order failed, client: {}, url: {}, csr: {}, {}", self, url, csr.len(), e);
                anyhow::anyhow!("finalize acme order failed, client: {}, url: {}, csr: {}, {}", self, url, csr.len(), e)
            })?;
        
        info!("finalize acme order success, client: {}, url: {}, csr: {}, response: {:?}", self, url, csr.len(), response);
        match response.status.as_str() {
            "valid" => Ok(response.certificate.ok_or_else(|| anyhow::anyhow!("No certificate URL in response"))?),
            _ => Err(anyhow::anyhow!("Order finalization failed: {}", response.status)),
        }
    }

    /// 发送 POST-as-GET 请求
    async fn post_as_get<R>(&self, url: &str) -> Result<R>
    where
        R: DeserializeOwned,
    {
        let nonce = self.get_nonce().await?;
        let jws = self.sign_request(url, &nonce, &"")?;
        
        let response = self.inner.http_client
            .post(url)
            .header("Content-Type", "application/jose+json")
            .json(&jws)
            .send()
            .await?;

        self.handle_response(response).await
    }

    /// 下载证书
    async fn download_certificate(&self, url: &str) -> Result<Vec<u8>> {
        info!("download acme certificate, client: {}, url: {}", self, url);
        
        let response = self.inner.http_client
            .get(url)
            .header("Accept", "application/pem-certificate-chain")
            .send()
            .await
            .map_err(|e| {
                error!("download acme certificate failed, client: {}, url: {}, {}", self, url, e);
                anyhow::anyhow!("download acme certificate failed: {}", e)
            })?;
        
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            error!("download certificate failed with status {}: {}", status, body);
            return Err(anyhow::anyhow!("Certificate download failed: {}", body));
        }

        let cert_data = response.bytes().await?.to_vec();
        info!("download acme certificate success, client: {}, url: {}", self, url);
        Ok(cert_data)
    }

    /// 创建新的订单
    pub async fn create_order(&self, domains: &[String]) -> Result<(Vec<String>, String)> {
        info!("create acme order, client: {}, domains: {}", self, domains.join(","));
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
        ).await
        .map_err(|e| {
            error!("create acme order failed, client: {}, domains: {}, {}", self, domains.join(","), e);
            anyhow::anyhow!("create acme order failed, client: {}, domains: {}, {}", self, domains.join(","), e)
        })?;

        info!("create acme order success, client: {}, domains: {}, response: {:?}", self, domains.join(","), response);

        // 返回授权URL列表和finalize URL
        Ok((response.authorizations, response.finalize))
    }

    fn generate_csr(&self, domains: &[String]) -> Result<(Vec<u8>, Vec<u8>)> {
        let rsa = Rsa::generate(2048)?;
        let pkey = PKey::from_rsa(rsa)?;
        
        let mut builder = X509ReqBuilder::new()?;
        builder.set_pubkey(&pkey)?;
        
        let mut name_builder = X509NameBuilder::new()?;
        name_builder.append_entry_by_text("CN", &domains[0])?;
        builder.set_subject_name(&name_builder.build())?;

        if !domains.is_empty() {
            let mut san = SubjectAlternativeName::new();
            for domain in domains {
                san.dns(domain);
            }
            let ext = san.build(&builder.x509v3_context(None))?;
            let mut stack = openssl::stack::Stack::new()?;
            stack.push(ext)?;
            builder.add_extensions(&stack)?;
        }

        builder.sign(&pkey, openssl::hash::MessageDigest::sha256())?;
        
        Ok((builder.build().to_der()?, pkey.private_key_to_pem_pkcs8()?))
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
            .header("Content-Type", "application/jose+json")
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
        info!("fetch acme nonce, client: {}", self);
        let response = self.inner.http_client
            .head(&self.inner.directory.new_nonce)
            .send()
            .await
            .map_err(|e| {
                error!("fetch acme nonce failed, client: {}, {}", self, e);
                anyhow::anyhow!("fetch acme nonce failed, client: {}, {}", self, e)
            })?;

        let nonce = response
            .headers()
            .get("Replay-Nonce")
            .ok_or_else(|| anyhow::anyhow!("No nonce found"))?
            .to_str()?
            .to_string();

        info!("fetch acme nonce success, client: {}, nonce: {}", self, nonce);

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
            let body = response.text().await?;
            error!("acme response error, status: {}, body: {}", status, body);
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
        
        let protected = if let Some(kid) = self.inner.account.kid() {
            // 已注册账户使用 kid
            serde_json::json!({
                "alg": "RS256",
                "kid": kid,
                "nonce": nonce,
                "url": url
            })
        } else {
            // 未注册账户使用 jwk
            serde_json::json!({
                "alg": "RS256",
                "nonce": nonce,
                "url": url,
                "jwk": {
                    "kty": "RSA",
                    "n": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.account().key().rsa()?.n().to_vec()),
                    "e": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.account().key().rsa()?.e().to_vec()),
                }
            })
        };
        
        let protected_str = serde_json::to_string(&protected)?;
        let protected_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(protected_str);
        
        let signing_input = format!("{}.{}", protected_b64, payload_b64);
        let mut signer = openssl::sign::Signer::new(
            openssl::hash::MessageDigest::sha256(),
            &self.account().key()
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

    /// 发送签名的POST请求，不需要响应体
    async fn signed_post_no_body<T: Serialize>(&self, url: &str, payload: &T) -> Result<()> {
        let nonce = self.get_nonce().await?;
        let jws = self.sign_request(url, &nonce, payload)?;
        
        let response = self.inner.http_client
            .post(url)
            .header("Content-Type", "application/jose+json")
            .json(&jws)
            .send()
            .await?;

        self.handle_response_no_body(response).await
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
    identifier: Identifier,
    status: String,
    expires: String,
    challenges: Vec<ChallengeResponse>,
}

#[derive(Debug, Deserialize)]
struct ChallengeResponse {
    #[serde(rename = "type")]
    type_: String,
    url: String,
    status: String,
    token: String,
}

#[derive(Debug, Deserialize)]
struct FinalizeResponse {
    status: String,
    certificate: Option<String>,
    #[serde(default)]
    error: Option<AcmeError>,
}

#[derive(Debug, Deserialize)]
struct AccountResponse {
    status: String,
    #[serde(default)]
    contact: Vec<String>,
    orders: Option<String>,
}