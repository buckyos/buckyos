use std::net::SocketAddr;
use async_trait::async_trait;
use crate::TunnelResult;
pub struct StreamRequest {
    pub dest_port : u16,
    pub dest_host : Option<String>, 
    pub dest_addr : Option<SocketAddr>,
    pub app_protocol: Option<String>,//一般可以通过dest_port来判断,但用request里也可以预设更准确的协议
    pub dest_url : Option<String>,

    pub source_addr : Option<SocketAddr>,
    pub source_mac : Option<String>,
    pub source_device_id: Option<String>,
    pub source_app_id: Option<String>,
    pub source_user_id: Option<String>, 
}

impl StreamRequest {
    pub fn new() -> Self {
        StreamRequest {
            dest_port: 0,
            dest_host: None,
            dest_addr: None,
            app_protocol: None,
            dest_url: None,
            source_addr: None,
            source_mac: None,
            source_device_id: None,
            source_app_id: None,
            source_user_id: None,
        }
    }
}

pub type DatagramRequest = StreamRequest;


pub trait StreamProbe {
    fn probe(&self, buffer: &[u8], request:&StreamRequest) -> TunnelResult<StreamRequest>;
}

pub trait DatagramProbe {
    fn probe(&self, buffer: &[u8], request:&DatagramRequest) -> TunnelResult<DatagramRequest>;
}

#[async_trait]
pub trait StreamSelector {
    //return stream_url
    async fn select(&self, request: StreamRequest) -> TunnelResult<String>;
}


