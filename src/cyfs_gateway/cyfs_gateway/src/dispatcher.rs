#![allow(unused)]
use cyfs_gateway_lib::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use log::*;
use url::Url;
use tokio::task;



type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone)]
pub struct ServiceDispatcher {
    //services: Arc<Mutex<Vec<UpstreamService>>>,
    config_source:Option<String>,
    config_version:u64,
    config: Arc<Mutex<HashMap<Url,DispatcherConfig>>>,
}

impl ServiceDispatcher {
    pub fn new(config: HashMap<Url,DispatcherConfig>) -> Self {
        Self {
            config_source: None,
            config_version: 0,
            config: Arc::new(Mutex::new(config)),
        }
    }

    pub async fn create_income_listener(&self, incoming: &Url) -> Result<Box<dyn StreamListener>> {
        let new_listener = create_listner_by_url(incoming).await.map_err(|e| {
            error!("create_income_listener failed, {}", e);
            Box::new(e)
        })?;
        Ok(new_listener)
    }

    pub async fn create_income_datagram_server(&self, incoming: &Url) -> Result<Box<dyn DatagramServerBox>> {
        let new_server = create_datagram_server_by_url(incoming).await.map_err(|e| {
            error!("create_income_datagram_server failed, {}", e);
            Box::new(e)
        })?;
        Ok(new_server)
    }

    pub async fn start_foward_service(&self, incoming: &Url, target: &Url) -> Result<()> {
        let incoming_category = get_protocol_category(incoming.scheme()).map_err(|e| {
            error!("start_foward_service failed, Invalid incoming protocol: {}", e);
            e
        })?;
        let target_category = get_protocol_category(target.scheme()).map_err(|e| {
            error!("start_foward_service failed, Invalid target protocol: {}", e);
            e
        })?;

        if incoming_category != target_category {
            error!("start_foward_service failed, incoming protocol and target protocol must be the same");
            return Err(Box::new(TunnelError::UnknowProtocol("incoming protocol and target protocol must be the same".to_string())));
        }

        let target_port = target.port().unwrap_or(incoming.port().unwrap_or(0));
        if target_port == 0 {
            error!("start_foward_service failed, target port is not specified");
            return Err(Box::new(TunnelError::UnknowProtocol("target port is not specified".to_string())));
        }

        match target_category {
            ProtocolCategory::Stream => {
                let listener = self.create_income_listener(incoming).await?;
                let incoming = incoming.clone();
                let target = target.clone();
                task::spawn(async move {
                    loop {
                        let accept_result = listener.accept().await;
                        if accept_result.is_err() {
                            warn!("stream forward service process accept failed: {}", accept_result.err().unwrap());
                            break;
                        }
                        let (mut income_stream, _) = accept_result.unwrap(); 
                        info!("stream forward service accept connection from {}", incoming);
                        let target_tunnel = get_tunnel(&target,None).await;
                        if target_tunnel.is_err() {
                            warn!("stream forward service process accept failed, get target tunnel failed: {}", target_tunnel.err().unwrap());
                            continue;
                        }
                        let target_tunnel = target_tunnel.unwrap();
                        let mut target_stream = target_tunnel.open_stream(target_port).await;
                        if target_stream.is_err() {
                            warn!("stream forward service forward connection failed, open target stream failed: {}", target_stream.err().unwrap());
                            continue;
                        }
                        let mut target_stream = target_stream.unwrap();
                        task::spawn(async move {
                            tokio::io::copy_bidirectional(income_stream.as_mut(),target_stream.as_mut()).await;
                        });
                        
                    }
                });
            },
            ProtocolCategory::Datagram => {
                let income_server = self.create_income_datagram_server(incoming).await?;
                let incoming = incoming.clone();
                let target = target.clone();
                type DatagramClientSession = Box<dyn DatagramClientBox>;
                type DatagramClientSessionMap = Arc<Mutex<HashMap<TunnelEndpoint,DatagramClientSession>>>;
                task::spawn(async move {
                    let mut buffer = vec![0u8; 1024*4];
                    let mut read_len:usize = 0;
                    let mut all_client_session:DatagramClientSessionMap = Arc::new(Mutex::new(HashMap::new()));
                    let mut source_ep:TunnelEndpoint;
                    loop {
                        let recv_result = income_server.recv_datagram(&mut buffer).await;
                        if recv_result.is_err() {
                            warn!("datagram forward service process recvfrom income_server failed: {}", recv_result.err().unwrap());
                            continue;
                        }
                        (read_len,source_ep) = recv_result.unwrap();
                        let mut all_sessions = all_client_session.lock().await;
                        let clientsession = all_sessions.get(&source_ep);
                        
                        if clientsession.is_some() {
                            let clientsession = clientsession.unwrap();
                            clientsession.send_datagram(&buffer[0..read_len]);
                            drop(all_sessions);
                        } else {
                            let target_tunnel = get_tunnel(&target,None).await;
                            if target_tunnel.is_err() {
                                warn!("datagram-forward create tunnel failed:{}", target_tunnel.err().unwrap());
                                continue;
                            }
                            let target_tunnel = target_tunnel.unwrap();
                            let datagram_client = target_tunnel.create_datagram_client(target_port).await;
                            if datagram_client.is_err() {
                                warn!("datagram-forward create datagram client failed: {}", datagram_client.err().unwrap());
                                continue;
                            }
                            info!("datagram-forward create a new client from {}", incoming);
                            let datagram_client = datagram_client.unwrap();
                            let _ = datagram_client.send_datagram(&buffer[0..read_len]).await;
                            all_sessions.insert(source_ep.clone(),datagram_client.clone_box());
                            drop(all_sessions);

                            let income_server2 = income_server.clone();
                            task::spawn(async move {
                                //store datagram_client_session
                                let mut buffer2 = vec![0u8; 1024*4];
                                let mut read_len2:usize = 0;
                                loop {
                                    //TODO: idel timeout,delete self from datagram_client_session
                                    let read_result = datagram_client.recv_datagram(&mut buffer2).await;
                                    if read_result.is_err() {
                                        warn!("datagram-forward recvfrom target failed: {}", read_result.err().unwrap());
                                        break;
                                    }
                                    read_len2 = read_result.unwrap();
                                    let send_result = income_server2.send_datagram(&source_ep,&buffer2[0..read_len2]).await;
                                    if send_result.is_err() {
                                        warn!("datagram-forward response to income_server failed: {}", send_result.err().unwrap());
                                        break;
                                    }
                                }
                            });
                        }     
                    }
                });
            }
        }

        Ok(())
    }

    pub async fn start(&self) {
        info!("Service dispatcher started");
        let config = self.config.lock().await;
        
        for (incomeing, target) in config.iter() {
            match &target.target {
                DispatcherTarget::Forward(target_url) => {
                    //TODO: store the task handle to stop it when the config is changed
                    match self.start_foward_service(incomeing, target_url).await {
                        Ok(_) => {
                            info!("Start foward service from {} to {} OK ", incomeing.to_string(), target_url.to_string());
                        },
                        Err(e) => {
                            error!("Start foward service from {} to {} failed, {}", incomeing.to_string(), target_url.to_string(), e);
                        }
                    }
                }
                DispatcherTarget::Server(server_id) => {
                    unimplemented!();
                    //looking for server config by server_id
                    //start server with config
                }
            }
        }
    }


    pub fn flush_new_config(&self, config: Option<DispatcherConfig>) {
        unimplemented!();
        // if config is None, then reload the config from the config_source
        //verify new config
        //calculate the difference between the new config and the old config
        //execuite the difference
    }
}

