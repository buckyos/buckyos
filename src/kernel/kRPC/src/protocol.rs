use std::net::IpAddr;
use async_trait::async_trait;
use serde::{Deserialize, Serialize,Serializer};
use serde_json::{Value};
use serde::ser::SerializeStruct;
use crate::RPCErrors;
pub enum RPCProtoclType {
    HttpPostJson,
}


#[derive(Debug,PartialEq)]
pub struct RPCRequest {
    pub method: String,
    pub params: Value,
    //0: seq,1:token(option),2:trace_id(option)
    //pub sys:  Option<Vec<Value>>,

    pub id:u64,
    pub token:Option<String>,
    pub trace_id:Option<String>,
}


impl RPCRequest {
    pub fn new(method:&str,params:Value) -> Self {
        RPCRequest {
            method: method.to_string(),
            params: params,
            id:0,
            token:None,
            trace_id:None,
        }
    }
    fn get_str_param_from_req(self: &RPCRequest, key: &str) -> Result<String, RPCErrors> {
        self.params
            .get(key)
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .ok_or(RPCErrors::ParseRequestError(format!(
                "Failed to get {} from params",
                key
            )))
    }

}

fn array_remove_none_value(array:&mut Vec<Value>) {
    let mut i = 0;
    while i < array.len() {
        if array[i].is_null() {
            array.remove(i);
        } else {
            i += 1;
        }
    }
}

impl Serialize for RPCRequest {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("RPCRequest", 2)?;
        state.serialize_field("method", &self.method)?;
        state.serialize_field("params", &self.params)?;
        let mut sys_vec = serde_json::json! {
            [self.id,self.token,self.trace_id]
        };
        array_remove_none_value(&mut sys_vec.as_array_mut().unwrap());
        state.serialize_field("sys", &sys_vec)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for RPCRequest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = Value::deserialize(deserializer)?;
        let method = v.get("method").ok_or(serde::de::Error::missing_field("method"))?;
        let method = method.as_str().ok_or(serde::de::Error::custom("method is not string"))?;
        let params = v.get("params").ok_or(serde::de::Error::missing_field("params"))?;

        let sys = v.get("sys");
        let mut seq:u64 = 0;
        let mut token:Option<String> = None;
        let mut trace_id:Option<String> = None;
        if sys.is_some() {
            let sys = sys.unwrap().as_array()
                .ok_or(serde::de::Error::custom("sys is not array"))?;

            if sys.len() > 0 {
                let _seq = sys[0].as_u64()
                    .ok_or(serde::de::Error::custom("sys[0] seq is not u64"))?;
                seq = _seq;
            }
            if sys.len() > 1 {
                let _token = sys[1].as_str()
                    .ok_or(serde::de::Error::custom("sys[1] token is not string"))?;
                token = Some(_token.to_string());
            }
            if sys.len() > 2 {
                let _trace_id = sys[2].as_str()
                    .ok_or(serde::de::Error::custom("sys[2] trace_id is not string"))?;
                trace_id = Some(_trace_id.to_string());
            }
        }  
        

        Ok(RPCRequest {
            method: method.to_string(),
            params: params.clone(),
            id:seq,
            token:token,
            trace_id:trace_id,
        })      
    }
}

#[derive(Debug, PartialEq)]
pub enum RPCResult {
    Success(Value),
    Failed(String),
}

#[derive(Debug,PartialEq)]
pub struct RPCResponse {
    pub result: RPCResult,

    pub seq:u64,
    pub token:Option<String>,
    pub trace_id:Option<String>,
}

impl RPCResponse {
    pub fn new(result:RPCResult,seq:u64) -> Self {
        RPCResponse {
            result: result,
            seq:seq,
            token:None,
            trace_id:None,
        }
    }
}

impl Serialize for RPCResponse {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match &self.result {
            RPCResult::Success(value) => {
                let mut state: <S as Serializer>::SerializeStruct = serializer.serialize_struct("RPCResponse", 1)?;
                state.serialize_field("result", &value)?;
                
                let mut sys_vec = serde_json::json! {
                    [self.seq,self.token,self.trace_id]
                };
                array_remove_none_value(&mut sys_vec.as_array_mut().unwrap());
                state.serialize_field("sys", &sys_vec)?;
                
                state.end()
            }
            RPCResult::Failed(err) => {
                let mut state = serializer.serialize_struct("RPCResponse", 1)?;
                state.serialize_field("error", &err)?;
                let mut sys_vec = serde_json::json! {
                    [self.seq,self.token,self.trace_id]
                };
                array_remove_none_value(&mut sys_vec.as_array_mut().unwrap());
                state.serialize_field("sys", &sys_vec)?;
                state.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for RPCResponse {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = Value::deserialize(deserializer)?;
        let sys = v.get("sys");
        let mut seq:u64 = 0;
        let mut token:Option<String> = None;
        let mut trace_id:Option<String> = None;
        if sys.is_some() {
            let sys = sys.unwrap().as_array()
                .ok_or(serde::de::Error::custom("sys is not array"))?;

            if sys.len() > 0 {
                let _seq = sys[0].as_u64()
                    .ok_or(serde::de::Error::custom("sys[0] seq is not u64"))?;
                seq = _seq;
            }
            if sys.len() > 1 {
                let _token = sys[1].as_str()
                    .ok_or(serde::de::Error::custom("sys[1] token is not string"))?;
                token = Some(_token.to_string());
            }
            if sys.len() > 2 {
                let _trace_id = sys[2].as_str()
                    .ok_or(serde::de::Error::custom("sys[2] trace_id is not string"))?;
                trace_id = Some(_trace_id.to_string());
            }
        } 
        
        if v.get("error").is_some() {
            Ok(RPCResponse {
                result: RPCResult::Failed(v.get("error").unwrap().as_str().unwrap().to_string()),
                seq:seq,
                token:token,
                trace_id:trace_id,
            })
        } else {
            Ok(RPCResponse {
                result: RPCResult::Success(v.get("result").unwrap().clone()),
                seq:seq,
                token:token,
                trace_id:trace_id,
            })
            
        }

    }
}

#[async_trait]
pub trait InnerServiceHandler {
    async fn handle_rpc_call(&self, req:RPCRequest,ip_from:IpAddr) -> Result<RPCResponse,RPCErrors>;
    async fn handle_http_get(&self, req_path:&str,ip_from:IpAddr) -> Result<String,RPCErrors>;
}

