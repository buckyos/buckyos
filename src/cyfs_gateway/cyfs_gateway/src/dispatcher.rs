#![allow(unused)]
use cyfs_gateway_lib::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
        let new_listener = create_listener_by_url(incoming).await.map_err(|e| {
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

    pub async fn start_selector_service(&self, incoming: &Url, probe_id:Option<String>, selector_id:String) -> Result<()> {
        info!("Will start selector service from {} to {} , with probe_id: {:?}", incoming, selector_id, probe_id);
        let incoming_category = get_protocol_category(incoming.scheme()).map_err(|e| {
            error!("start_selector_service failed, Invalid incoming protocol: {}", e);
            e
        })?;

        match incoming_category {
            ProtocolCategory::Stream => {
                let listener = self.create_income_listener(incoming).await?;
                let incoming = incoming.clone();
                let probe_id = probe_id.clone();
                let selector_id = selector_id.clone();
                task::spawn(async move {
                    loop {
                        let accept_result = listener.accept().await;
                        if accept_result.is_err() {
                            warn!("stream forward-selector service process accept failed: {}", accept_result.err().unwrap());
                            break;
                        }
                        let (mut income_stream, _) = accept_result.unwrap(); 
                        info!("stream forward-selector service accept connection from {}", incoming);
                        let probe_id = probe_id.clone();
                        let selector_id = selector_id.clone();
                        task::spawn(async move {
                            let this_probe:Option<Box<dyn StreamProbe + Send>>;
                            if probe_id.is_some() {
                                let probe = get_stream_probe(probe_id.unwrap().as_str());
                                if probe.is_err() {
                                    warn!("stream forward-selector service get probe failed: {}", probe.err().unwrap());
                                    return;
                                }
                                this_probe = Some(probe.unwrap());
                            } else {
                                this_probe = None;
                            }
                            let this_selector = get_stream_selector(selector_id.as_str());
                            if this_selector.is_err() {
                                warn!("stream forward-selector service get selector failed: {}", this_selector.err().unwrap());
                                return;
                            }
                            let this_selector = this_selector.unwrap();

                            let mut probe_buffer:[u8;1024*4] = [0;1024*4];
                            let mut read_len:usize = 0;
                            let mut stream_request = StreamRequest::new();
                            if this_probe.is_some() {
                                
                                let read_ret = income_stream.read(&mut probe_buffer).await;
                                if read_ret.is_err() {
                                    warn!("stream forward-selector service  read probe buffer failed: {}", read_ret.err().unwrap());
                                    return;
                                }
                                read_len = read_ret.unwrap();
                                if read_len == 0 {
                                    warn!("stream forward-selector service  read probe buffer failed: {}", read_len);
                                    return;
                                }
                                let probe = this_probe.as_ref().unwrap();
                                probe.probe(&probe_buffer[0..read_len], &mut stream_request);
                            }
                            let target_url = this_selector.select(stream_request).await;
                            if target_url.is_err() {
                                warn!("stream forward-selector service select target url failed: {}", target_url.err().unwrap());
                                return;
                            }
                            let target_url = target_url.unwrap();
                            let target_url = Url::parse(&target_url);
                            if target_url.is_err() {
                                warn!("stream forward-selector service parse target url failed: {}", target_url.err().unwrap());
                                return;
                            }
                            let target_url = target_url.unwrap();
                            let mut target_stream = open_stream_by_url(&target_url).await;
                            if target_stream.is_err() {
                                warn!("stream forward-selector service forward connection failed, open target stream failed: {}", target_stream.err().unwrap());
                                return;
                            }
                            let mut target_stream = target_stream.unwrap();
                            if read_len > 0 {
                                let write_result = target_stream.write_all(&probe_buffer[0..read_len]).await;
                                if write_result.is_err() {
                                    warn!("stream forward-selector service write probe buffer failed: {}", write_result.err().unwrap());
                                    return;
                                }
                            }
                            tokio::io::copy_bidirectional(income_stream.as_mut(),target_stream.as_mut()).await;
                        });
                        
                    }
                });
            },
            ProtocolCategory::Datagram => {
                return Err(Box::new(TunnelError::UnknownProtocol("Datagram protocol not supported".to_string())));
            }
        }

        Ok(())
    }


    pub async fn start_forward_service(&self, incoming: &Url, target: &Url) -> Result<()> {
        info!("Will start forward service from {} to {}", incoming, target);

        let incoming_category = get_protocol_category(incoming.scheme()).map_err(|e| {
            error!("start_forward_service failed, Invalid incoming protocol: {}", e);
            e
        })?;
        let target_category = get_protocol_category(target.scheme()).map_err(|e| {
            error!("start_forward_service failed, Invalid target protocol: {}", e);
            e
        })?;

        if incoming_category != target_category {
            let msg = format!("start_forward_service failed, incoming protocol and target protocol must be the same: {} {}", incoming, target);
            error!("{}", msg);
            return Err(Box::new(TunnelError::UnknownProtocol(msg)));
        }

        let target_port = target.port().unwrap_or(incoming.port().unwrap_or(0));
        if target_port == 0 {
            let msg = format!("start_forward_service failed, target port is not specified: {}", target);
            error!("{}", msg);
            return Err(Box::new(TunnelError::UnknownProtocol(msg)));
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
                        let mut target_stream = open_stream_by_url(&target).await;
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
                            let datagram_client = create_datagram_client_by_url(&target).await;
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
        
        for (incoming, target) in config.iter() {
            match &target.target {
                DispatcherTarget::Forward(target_url) => {
                    //TODO: store the task handle to stop it when the config is changed
                    match self.start_forward_service(incoming, target_url).await {
                        Ok(_) => {
                            info!("Start forward service from {} to {} OK ", incoming.to_string(), target_url.to_string());
                        },
                        Err(e) => {
                            error!("Start forward service from {} to {} failed, {}", incoming.to_string(), target_url.to_string(), e);
                        }
                    }
                }
                DispatcherTarget::Server(server_id) => {
                    info!("dispatcher from {} to server {}", incoming.to_string(), server_id);
                    //looking for server config by server_id
                    //start server with config
                }
                DispatcherTarget::Selector(selector_id) => {
                    info!("dispatcher from {} to selector {}", incoming.to_string(), selector_id);
                    let start_result = self.start_selector_service(incoming, None, selector_id.clone()).await;
                    if start_result.is_err() {
                        error!("start selector service from {} to {} failed, {}", incoming.to_string(), selector_id, start_result.err().unwrap());
                    } else {
                        info!("start selector service from {} to {} OK", incoming.to_string(), selector_id);
                    }
                }
                DispatcherTarget::ProbeSelector(probe_id, selector_id) => {
                    info!("dispatcher from {} to probe_selector {}", incoming.to_string(), probe_id);
                    let start_result = self.start_selector_service(incoming, Some(probe_id.clone()), selector_id.clone()).await;
                    if start_result.is_err() {
                        error!("start probe_selector service from {} to {} failed, {}", incoming.to_string(), selector_id, start_result.err().unwrap());
                    } else {
                        info!("start probe_selector service from {} to {} OK", incoming.to_string(), selector_id);
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dispatcher() {
        let dispatcher = ServiceDispatcher::new(HashMap::new());
        dispatcher.start().await;
    }
}