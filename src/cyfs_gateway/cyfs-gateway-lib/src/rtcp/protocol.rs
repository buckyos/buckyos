
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
use anyhow::Result;

pub const DEFAULT_RTCP_STACK_PORT: u16 = 2980;


#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RTcpTargetStackEP {
    pub did: DID,
    pub stack_port: u16
}

impl RTcpTargetStackEP {
    pub fn new(target_did: DID, stack_port: u16) -> Result<Self> {
        Ok(RTcpTargetStackEP {
            did: target_did,
            stack_port
        })
    }
}

// xxx.dev.did:2980 or xxx:2980 
pub(crate) fn parse_rtcp_stack_id(stack_id: &str) -> Option<RTcpTargetStackEP> {
    let stack_port = DEFAULT_RTCP_STACK_PORT;
    //let mut target_host_name = stack_id.to_string();
    let target_did = DID::from_str(stack_id);
    if target_did.is_err() {
        return None;
    }
    let target_did = target_did.unwrap();

    let target = RTcpTargetStackEP {
        did: target_did,
        stack_port: stack_port,
    };

    return Some(target);
}