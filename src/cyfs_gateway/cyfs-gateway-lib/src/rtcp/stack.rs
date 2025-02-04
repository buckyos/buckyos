use super::package::*;
use super::protocol::*;
use super::stream_helper::RTcpStreamBuildHelper;
use super::tunnel::RTcpTunnel;
use super::tunnel_map::RTcpTunnelMap;
use crate::tunnel::{DatagramServerBox, StreamListener, TunnelBox, TunnelBuilder};
use crate::{TunnelError, TunnelResult};
use anyhow::Result;
use async_trait::async_trait;
use buckyos_kit::buckyos_get_unix_timestamp;
use hex::ToHex;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use log::*;
use name_client::*;
use name_lib::*;
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::pin::Pin;
use tokio::net::{TcpListener, TcpStream};
use tokio::task;
use url::Url;
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};
use super::dispatcher::RTcpDispatcherManager;

#[derive(Clone)]
pub struct RTcpStack {
    tunnel_map: RTcpTunnelMap,
    stream_helper: RTcpStreamBuildHelper,
    dispatcher_manager: RTcpDispatcherManager,

    tunnel_port: u16,
    this_device_hostname: String, //name or did
    this_device_ed25519_sk: Option<EncodingKey>,
    this_device_x25519_sk: Option<StaticSecret>,
}

impl RTcpStack {
    pub fn new(
        this_device_hostname: String,
        port: u16,
        private_key_pkcs8_bytes: Option<[u8; 48]>,
    ) -> RTcpStack {
        let mut this_device_x25519_sk = None;
        let mut this_device_ed25519_sk = None;
        if private_key_pkcs8_bytes.is_some() {
            let private_key_pkcs8_bytes = private_key_pkcs8_bytes.unwrap();
            //info!("rtcp stack ed25519 private_key pkcs8 bytes: {:?}",private_key_pkcs8_bytes);
            let encoding_key = EncodingKey::from_ed_der(&private_key_pkcs8_bytes);
            this_device_ed25519_sk = Some(encoding_key);

            let private_key_bytes = from_pkcs8(&private_key_pkcs8_bytes).unwrap();
            //info!("rtcp stack ed25519 private_key  bytes: {:?}",private_key_bytes);

            let x25519_private_key =
                ed25519_to_curve25519::ed25519_sk_to_curve25519(private_key_bytes);
            //info!("rtcp stack x25519 private_key_bytes: {:?}",x25519_private_key);
            this_device_x25519_sk = Some(x25519_dalek::StaticSecret::from(x25519_private_key));
        }

        let result = RTcpStack {
            tunnel_map: RTcpTunnelMap::new(),
            stream_helper: RTcpStreamBuildHelper::new(),
            dispatcher_manager: RTcpDispatcherManager::new(),

            tunnel_port: port,
            this_device_hostname,
            this_device_ed25519_sk: this_device_ed25519_sk, //for sign tunnel token
            this_device_x25519_sk: this_device_x25519_sk,   //for decode tunnel token from remote
        };
        return result;
    }

    // return (tunnel_token,aes_key,my_public_bytes)
    async fn generate_tunnel_token(
        &self,
        target_hostname: String,
    ) -> Result<(String, [u8; 32], [u8; 32]), TunnelError> {
        if self.this_device_ed25519_sk.is_none() {
            return Err(TunnelError::DocumentError(
                "this device ed25519 sk is none".to_string(),
            ));
        }

        let (auth_key, remote_did_id) = resolve_ed25519_auth_key(target_hostname.as_str())
            .await
            .map_err(|op| {
                let msg = format!(
                    "cann't resolve target device {} auth key: {}",
                    target_hostname.as_str(),
                    op
                );
                error!("{}", msg);
                TunnelError::DocumentError(msg)
            })?;

        //info!("remote ed25519 auth_key: {:?}",auth_key);
        let remote_x25519_pk = ed25519_to_curve25519::ed25519_pk_to_curve25519(auth_key);
        //info!("remote x25519 pk: {:?}",remote_x25519_pk);

        let my_secret = EphemeralSecret::random();
        let my_public = PublicKey::from(&my_secret);
        let my_public_bytes = my_public.to_bytes();
        let my_public_hex = my_public.encode_hex();
        //info!("my_public_hex: {:?}",my_public_hex);
        let aes_key = RTcpStack::generate_aes256_key(my_secret, remote_x25519_pk);
        //info!("aes_key: {:?}",aes_key);
        //create jwt by tunnel token payload
        let tunnel_token_payload = TunnelTokenPayload {
            to: remote_did_id,
            from: self.this_device_hostname.clone(),
            xpub: my_public_hex,
            exp: buckyos_get_unix_timestamp() + 3600 * 2,
        };
        info!("send tunnel_token_payload: {:?}", tunnel_token_payload);
        let payload = serde_json::to_value(&tunnel_token_payload).map_err(|op| {
            TunnelError::ReasonError(format!("encode tunnel token payload error:{}", op))
        })?;

        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = None;
        header.typ = None;
        let tunnel_token = encode(
            &header,
            &payload,
            &self.this_device_ed25519_sk.as_ref().unwrap(),
        );
        if tunnel_token.is_err() {
            let err_str = tunnel_token.err().unwrap().to_string();
            return Err(TunnelError::ReasonError(err_str));
        }
        let tunnel_token = tunnel_token.unwrap();

        Ok((tunnel_token, aes_key, my_public_bytes))
    }

    fn generate_aes256_key(
        this_private_key: EphemeralSecret,
        x25519_public_key: [u8; 32],
    ) -> [u8; 32] {
        //info!("will create share sec with remote x25519 pk: {:?}",x25519_public_key);
        let x25519_public_key = x25519_dalek::PublicKey::from(x25519_public_key);
        let shared_secret = this_private_key.diffie_hellman(&x25519_public_key);

        let mut hasher = Sha256::new();
        hasher.update(shared_secret.as_bytes());
        let key_bytes = hasher.finalize();
        return key_bytes.try_into().unwrap();
        //return shared_secret.as_bytes().clone();
    }

    pub async fn decode_tunnel_token(
        this_private_key: &StaticSecret,
        token: String,
        from_hostname: String,
    ) -> Result<([u8; 32], [u8; 32]), TunnelError> {
        let (ed25519_pk, _from_did) = resolve_ed25519_auth_key(from_hostname.as_str())
            .await
            .map_err(|op| {
                TunnelError::DocumentError(format!(
                    "cann't resolve from device {} auth key:{}",
                    from_hostname.as_str(),
                    op
                ))
            })?;

        let from_public_key = DecodingKey::from_ed_der(&ed25519_pk);

        let tunnel_token_payload = decode::<TunnelTokenPayload>(
            token.as_str(),
            &from_public_key,
            &Validation::new(Algorithm::EdDSA),
        );
        if tunnel_token_payload.is_err() {
            return Err(TunnelError::DocumentError(
                "decode tunnel token error".to_string(),
            ));
        }
        let tunnel_token_payload = tunnel_token_payload.unwrap();
        let tunnel_token_payload = tunnel_token_payload.claims;
        //info!("tunnel_token_payload: {:?}",tunnel_token_payload);
        let remomte_x25519_pk = hex::decode(tunnel_token_payload.xpub).unwrap();

        let remomte_x25519_pk: [u8; 32] = remomte_x25519_pk.try_into().map_err(|_op| {
            let msg = format!("decode remote x25519 hex error");
            error!("{}", msg);
            TunnelError::ReasonError(msg)
        })?;

        //info!("remomte_x25519_pk: {:?}",remomte_x25519_pk);
        let aes_key = RTcpStack::get_aes256_key(this_private_key, remomte_x25519_pk.clone());
        //info!("aes_key: {:?}",aes_key);
        Ok((aes_key, remomte_x25519_pk))
    }

    fn get_aes256_key(
        this_private_key: &StaticSecret,
        remote_x25519_auth_key: [u8; 32],
    ) -> [u8; 32] {
        //info!("will get share sec with remote x25519 temp pk: {:?}",remote_x25519_auth_key);
        let x25519_public_key = x25519_dalek::PublicKey::from(remote_x25519_auth_key);
        let shared_secret = this_private_key.diffie_hellman(&x25519_public_key);

        let mut hasher = Sha256::new();
        hasher.update(shared_secret.as_bytes());
        let key_bytes = hasher.finalize();
        return key_bytes.try_into().unwrap();
    }

    pub async fn start(&mut self) -> TunnelResult<()> {
        // create a tcp listener for tunnel
        let bind_addr = format!("0.0.0.0:{}", self.tunnel_port);
        let rtcp_listener = TcpListener::bind(&bind_addr).await.map_err(|e| {
            let msg = format!("bind rtcp listener error:{}", e);
            error!("{}", msg);
            TunnelError::BindError(msg)
        })?;

        info!(
            "RTcp stack {} start ok: {}",
            self.this_device_hostname.as_str(),
            bind_addr
        );

        let this = self.clone();
        task::spawn(async move {
            loop {
                let (stream, addr) = rtcp_listener.accept().await.unwrap();
                info!("RTcp stack accept new tcp stream from {}", addr.clone());

                let this = this.clone();
                task::spawn(async move {
                    this.process_new_income_stream(stream, addr).await;
                });
            }
        });

        Ok(())
    }

    async fn process_new_income_stream(&self, mut stream: TcpStream, addr: SocketAddr) {
        let source_info = addr.to_string();
        let first_package =
            RTcpTunnelPackage::read_package(Pin::new(&mut stream), true, source_info.as_str())
                .await;
        if first_package.is_err() {
            error!(
                "Read first package error: {}, {}",
                addr,
                first_package.err().unwrap()
            );
            return;
        }

        debug!(
            "RTcp stream {} read first package ok",
            self.this_device_hostname.as_str()
        );
        let package = first_package.unwrap();
        match package {
            RTcpTunnelPackage::HelloStream(session_key) => {
                info!(
                    "RTcp stack {} accept new stream: {}, {}",
                    self.this_device_hostname.as_str(),
                    addr,
                    session_key
                );
                self.on_new_stream(stream, session_key).await;
            }
            RTcpTunnelPackage::Hello(hello_package) => {
                info!(
                    "RTcp stack {} accept new tunnel: {}, {} -> {}",
                    self.this_device_hostname.as_str(),
                    addr,
                    hello_package.body.from_id,
                    hello_package.body.to_id
                );

                self.on_new_tunnel(stream, hello_package).await;
            }
            _ => {
                error!("Unsupported first package type for rtcp stack: {}", addr);
            }
        }
    }

    async fn on_new_stream(&self, stream: TcpStream, session_key: String) {
        // find waiting ropen stream by session_key
        let real_key = format!(
            "{}_{}",
            self.this_device_hostname.as_str(),
            session_key.as_str()
        );

        self.stream_helper
            .notify_ropen_stream(stream, real_key.as_str())
            .await;
    }

    async fn on_new_tunnel(&self, stream: TcpStream, hello_package: RTcpHelloPackage) {
        // decode hello.body.tunnel_token
        if hello_package.body.tunnel_token.is_none() {
            error!("hello.body.tunnel_token is none");
            return;
        }
        let token = hello_package.body.tunnel_token.as_ref().unwrap().clone();
        let aes_key = RTcpStack::decode_tunnel_token(
            &self.this_device_x25519_sk.as_ref().unwrap(),
            token,
            hello_package.body.from_id.clone(),
        )
        .await;
        if aes_key.is_err() {
            error!("decode tunnel token error:{}", aes_key.err().unwrap());
            return;
        }

        let (aes_key, random_pk) = aes_key.unwrap();
        let target = RTcpTargetStackId::new(
            hello_package.body.from_id.as_str(),
            hello_package.body.my_port,
        );
        if target.is_err() {
            error!("parser remote did error:{}", target.err().unwrap());
            return;
        }
        let target = target.unwrap();
        let tunnel = RTcpTunnel::new(
            self.stream_helper.clone(),
            self.dispatcher_manager.clone(),
            self.this_device_hostname.clone(),
            &target,
            false,
            stream,
            aes_key,
            random_pk,
        );

        let tunnel_key = format!(
            "{}_{}",
            self.this_device_hostname.as_str(),
            hello_package.body.from_id.as_str()
        );
        {
            //info!("accept tunnel from {} try get lock",hello_package.body.from_id.as_str());
            self.tunnel_map
                .on_new_tunnel(&tunnel_key, tunnel.clone())
                .await;
            // info!("Accept tunnel from {}", hello_package.body.from_id.as_str());
        }

        info!(
            "Tunnel {} accept from {} OK,start running",
            hello_package.body.from_id.as_str(),
            tunnel_key.as_str()
        );
        tunnel.run().await;

        info!("Tunnel {} end", tunnel_key.as_str());

        self.tunnel_map.remove_tunnel(&tunnel_key).await;
    }
}

#[async_trait]
impl TunnelBuilder for RTcpStack {
    async fn create_tunnel(
        &self,
        tunnel_stack_id: Option<&str>,
    ) -> TunnelResult<Box<dyn TunnelBox>> {
        // lookup existing tunnel and resue it
        if tunnel_stack_id.is_none() {
            return Err(TunnelError::ReasonError(
                "rtcp target stack id is none".to_string(),
            ));
        }
        let tunnel_stack_id = tunnel_stack_id.unwrap();
        let target = parse_rtcp_stack_id(tunnel_stack_id);
        if target.is_none() {
            return Err(TunnelError::ConnectError(format!(
                "invalid target url:{:?}",
                target
            )));
        }
        let target: RTcpTargetStackId = target.unwrap();
        let target_id_str = target.get_id_str();

        let tunnel_key = format!(
            "{}_{}",
            self.this_device_hostname.as_str(),
            target_id_str.as_str()
        );
        debug!(
            "will create tunnel to {} ,tunnel key is {},try reuse",
            target_id_str.as_str(),
            tunnel_key.as_str()
        );

        // First check if the tunnel already exists, then we can reuse it
        let tunnels = self.tunnel_map.tunnel_map().clone();
        let mut all_tunnel = tunnels.lock().await;
        let tunnel = all_tunnel.get(tunnel_key.as_str());
        if tunnel.is_some() {
            debug!("Reuse tunnel {}", tunnel_key.as_str());
            return Ok(Box::new(tunnel.unwrap().clone()));
        }

        // 1） resolve target auth-key and ip (rtcp base on tcp,so need ip)

        let device_ip = resolve_ip(target_id_str.as_str()).await;
        if device_ip.is_err() {
            warn!(
                "cann't resolve target device {} ip.",
                target_id_str.as_str()
            );
            return Err(TunnelError::ConnectError(format!(
                "cann't resolve target device {} ip.",
                target_id_str.as_str()
            )));
        }
        let device_ip = device_ip.unwrap();
        let port = target.stack_port;
        let remote_addr = format!("{}:{}", device_ip, port);

        info!(
            "Will open tunnel to {}, target addr is {}",
            target_id_str.as_str(),
            remote_addr.as_str()
        );

        // connect to target
        let tunnel_stream = tokio::net::TcpStream::connect(remote_addr.clone()).await;
        if tunnel_stream.is_err() {
            warn!(
                "connect to {} error:{}",
                remote_addr,
                tunnel_stream.err().unwrap()
            );
            return Err(TunnelError::ConnectError(format!(
                "connect to {} error.",
                remote_addr
            )));
        }
        // create tunnel token
        let (tunnel_token, aes_key, random_pk) = self
            .generate_tunnel_token(target_id_str.clone())
            .await
            .map_err(|e| {
                let msg = format!("generate tunnel token error: {}, {}", target_id_str, e);
                error!("{}", msg);
                e
            })?;

        // send hello to target
        let mut tunnel_stream = tunnel_stream.unwrap();
        let hello_package = RTcpHelloPackage::new(
            0,
            self.this_device_hostname.clone(),
            target_id_str.clone(),
            self.tunnel_port,
            Some(tunnel_token),
        );
        let send_result =
            RTcpTunnelPackage::send_package(Pin::new(&mut tunnel_stream), hello_package).await;
        if send_result.is_err() {
            warn!(
                "send hello package to {} error:{}",
                remote_addr,
                send_result.err().unwrap()
            );
            return Err(TunnelError::ConnectError(format!(
                "send hello package to {} error.",
                remote_addr
            )));
        }

        // create tunnel and add to map
        let tunnel = RTcpTunnel::new(
            self.stream_helper.clone(),
            self.dispatcher_manager.clone(),
            self.this_device_hostname.clone(),
            &target,
            true,
            tunnel_stream,
            aes_key,
            random_pk,
        );
        all_tunnel.insert(tunnel_key.clone(), tunnel.clone());
        info!(
            "create tunnel {} ok, remote addr is {}",
            tunnel_key.as_str(),
            remote_addr.as_str()
        );
        drop(all_tunnel);

        let result: TunnelResult<Box<dyn TunnelBox>> = Ok(Box::new(tunnel.clone()));
        let tunnel_map = self.tunnel_map.clone();
        task::spawn(async move {
            info!(
                "RTcp tunnel {} established, tunnel running",
                tunnel_key.as_str()
            );
            tunnel.run().await;

            // remove tunnel from manager
            tunnel_map.remove_tunnel(&tunnel_key).await;

            info!("RTcp tunnel {} end", tunnel_key.as_str());
        });

        return result;
    }

    async fn create_stream_listener(
        &self,
        bind_url: &Url,
    ) -> TunnelResult<Box<dyn StreamListener>> {
        let dispatcher = self.dispatcher_manager.new_stream_dispatcher(bind_url)?;
        Ok(Box::new(dispatcher) as Box<dyn StreamListener>)
    }

    async fn create_datagram_server(
        &self,
        bind_url: &Url,
    ) -> TunnelResult<Box<dyn DatagramServerBox>> {
        let dispatcher = self.dispatcher_manager.new_datagram_dispatcher(bind_url)?;
        Ok(Box::new(dispatcher) as Box<dyn DatagramServerBox>)
    }
}
