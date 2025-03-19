use std::net::SocketAddr;
use async_trait::async_trait;
use crate::TunnelResult;

#[derive(Clone)]
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

pub struct HttpsSniProbe;

impl HttpsSniProbe {
    // 解析TLS Client Hello中的SNI
    fn extract_sni(buffer: &[u8]) -> Option<String> {
        // 检查是否是TLS握手消息
        if buffer.len() < 5 || buffer[0] != 0x16 || buffer[1] != 0x03 {
            return None;
        }

        let mut pos = 43; // 跳过TLS记录头和Client Hello固定部分
        if buffer.len() <= pos {
            return None;
        }

        // 跳过Session ID
        if pos < buffer.len() {
            let session_id_len = buffer[pos] as usize;
            pos += 1 + session_id_len;
        }

        // 跳过Cipher Suites
        if pos + 2 <= buffer.len() {
            let cipher_len = ((buffer[pos] as usize) << 8) | (buffer[pos + 1] as usize);
            pos += 2 + cipher_len;
        }

        // 跳过Compression Methods
        if pos + 1 <= buffer.len() {
            let comp_len = buffer[pos] as usize;
            pos += 1 + comp_len;
        }

        // 解析扩展
        if pos + 2 <= buffer.len() {
            let extensions_len = ((buffer[pos] as usize) << 8) | (buffer[pos + 1] as usize);
            pos += 2;
            let extensions_end = pos + extensions_len;

            while pos + 4 <= extensions_end {
                let ext_type = ((buffer[pos] as u16) << 8) | (buffer[pos + 1] as u16);
                let ext_len = ((buffer[pos + 2] as usize) << 8) | (buffer[pos + 3] as usize);
                pos += 4;

                // SNI 扩展类型为 0
                if ext_type == 0 && pos + ext_len <= buffer.len() {
                    // 解析SNI内容
                    if ext_len > 5 {
                        let sni_len = ((buffer[pos + 3] as usize) << 8) | (buffer[pos + 4] as usize);
                        if pos + 5 + sni_len <= buffer.len() {
                            return String::from_utf8(buffer[pos+5..pos+5+sni_len].to_vec()).ok();
                        }
                    }
                }
                pos += ext_len;
            }
        }
        None
    }
}

impl StreamProbe for HttpsSniProbe {
    fn probe(&self, buffer: &[u8], request: &StreamRequest) -> TunnelResult<StreamRequest> {
        let mut new_request:StreamRequest = request.clone();
        
        if let Some(hostname) = Self::extract_sni(buffer) {
            new_request.dest_host = Some(hostname);
            new_request.app_protocol = Some("https".to_string());
        }
        
        Ok(new_request)
    }
}


