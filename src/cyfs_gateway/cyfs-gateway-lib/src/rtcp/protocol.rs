
/*
tunnel的控制协议
二进制头：2+1+4=7字节
len: u16,如果是0则表示该Package是HelloStream包，后面是32字节的session_key
json_pos:u8, json数据的起始位置
cmd:u8,命令类型
seq:u32, 序列号
package:String， data in json format

控制协议分为如下类型：

// 建立tunnel，tunnel建立后，client需要立刻发送build包，用以确定该tunnel的信息
{
cmd:hello
from_id: string,
to_id: string,
test_port:u16
seession_key:option<string> （用对方公钥加密的key,并有自己的签名）
}
后续所有命令都用tunel key 对称加密
{
cmd:hello_ack
test_result:bool
}


{
cmd:ping

}
{
cmd:ping_resp
}


//因为无法与对端建立直连，因此通过该命令，要求对方建立反连，效果相当于命令发起方主动连接target
//并不使用直接复用当前tunnel+rebuild的方式,是想提高一些扩展性
//要求对端返连自己的端口
{
cmd:ropen
session_key:string （32个字节的随机字符串，第1，2个字符是byte 0）
target:Url,like tcp://_:123
}

{
cmd:ropen_resp
result:u32
}

*/

use name_lib::DID;
use url::Url;
use anyhow::Result;

pub const DEFAULT_RTCP_STACK_PORT: u16 = 2980;

#[derive(Debug, Clone, PartialEq)]
pub enum RTcpTargetId {
    DeviceName(String),
    DeviceDid(DID),
}

impl RTcpTargetId {
    pub fn from_str(value: &str) -> Option<Self> {
        if value.ends_with(".did") {
            let did = DID::from_host_name(value)?;
            return Some(RTcpTargetId::DeviceDid(did));
        }
        if value.starts_with("did:") {
            let did = DID::from_str(value)?;
            return Some(RTcpTargetId::DeviceDid(did));
        }
        Some(RTcpTargetId::DeviceName(value.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RTcpTarget {
    pub id: RTcpTargetId,
    pub stack_port: u16,
    pub target_port: u16,
}

impl RTcpTarget {
    pub fn new(hostname: &str, stack_port: u16, target_port: u16) -> Result<Self> {
        let id = RTcpTargetId::from_str(hostname);
        if id.is_none() {
            return Err(anyhow::anyhow!("invalid hostname:{}", hostname));
        }
        Ok(RTcpTarget {
            id: id.unwrap(),
            stack_port,
            target_port,
        })
    }

    pub fn from_did_str(did: String) -> Self {
        let did = DID::from_str(&did).unwrap();
        RTcpTarget {
            id: RTcpTargetId::DeviceDid(did),
            stack_port: DEFAULT_RTCP_STACK_PORT,
            target_port: 80,
        }
    }

    pub fn from_hostname(name: String) -> Self {
        RTcpTarget {
            id: RTcpTargetId::DeviceName(name),
            stack_port: DEFAULT_RTCP_STACK_PORT,
            target_port: 80,
        }
    }

    pub fn get_id_str(&self) -> String {
        match self.id {
            RTcpTargetId::DeviceName(ref name) => name.clone(),
            RTcpTargetId::DeviceDid(ref did) => did.to_host_name(),
        }
    }
}

pub(crate) fn parse_rtcp_url(url: &str) -> Option<RTcpTarget> {
    let url = Url::parse(url);
    if url.is_err() {
        return None;
    }
    let url = url.unwrap();
    if url.scheme() != "rtcp" {
        return None;
    }

    let mut stack_port = DEFAULT_RTCP_STACK_PORT;
    if url.username().len() > 0 {
        let _port = url.username().parse::<u16>();
        if _port.is_ok() {
            stack_port = _port.unwrap();
        }
    }

    let host = url.host();
    if host.is_none() {
        return None;
    }
    let host = host.unwrap();
    let result_id: RTcpTargetId;

    let target_did = DID::from_host_name(host.to_string().as_str());
    if target_did.is_some() {
        result_id = RTcpTargetId::DeviceDid(target_did.unwrap());
    } else {
        result_id = RTcpTargetId::DeviceName(host.to_string());
    }

    let target = RTcpTarget {
        id: result_id,
        stack_port: stack_port,
        target_port: url.port().unwrap_or(80),
    };

    return Some(target);
}