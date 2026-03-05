use crate::RPCErrors;
use async_trait::async_trait;
use buckyos_kit::buckyos_get_unix_timestamp;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use std::net::IpAddr;
pub enum RPCProtoclType {
    HttpPostJson,
}

#[derive(Debug, PartialEq)]
pub struct RPCRequest {
    pub method: String,
    pub params: Value,
    //0: seq,1:token(option),2:trace_id(option)
    //pub sys:  Option<Vec<Value>>,
    pub seq: u64,
    pub token: Option<String>,
    pub trace_id: Option<String>,
}

impl RPCRequest {
    pub fn new(method: &str, params: Value) -> Self {
        RPCRequest {
            method: method.to_string(),
            params: params,
            seq: 0,
            token: None,
            trace_id: None,
        }
    }

    pub fn get_str_param(self: &RPCRequest, key: &str) -> Result<String, RPCErrors> {
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



impl Serialize for RPCRequest {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("RPCRequest", 2)?;
        state.serialize_field("method", &self.method)?;
        state.serialize_field("params", &self.params)?;
        let mut sys_vec = vec![Value::from(self.seq)];
        if let Some(token) = self.token.as_ref() {
            sys_vec.push(Value::from(token.clone()));
        } else if self.trace_id.is_some() {
            // keep slot for token when trace_id exists
            sys_vec.push(Value::Null);
        }
        if let Some(trace_id) = self.trace_id.as_ref() {
            sys_vec.push(Value::from(trace_id.clone()));
        }
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
        let method = v
            .get("method")
            .ok_or(serde::de::Error::missing_field("method"))?;
        let method = method
            .as_str()
            .ok_or(serde::de::Error::custom("method is not string"))?;
        let params = v
            .get("params")
            .ok_or(serde::de::Error::missing_field("params"))?;

        let sys = v.get("sys");
        let mut seq: u64 = 0;
        let mut token: Option<String> = None;
        let mut trace_id: Option<String> = None;
        if sys.is_some() {
            let sys = sys
                .unwrap()
                .as_array()
                .ok_or(serde::de::Error::custom("sys is not array"))?;

            if sys.len() > 0 {
                let _seq = sys[0]
                    .as_u64()
                    .ok_or(serde::de::Error::custom("sys[0] seq is not u64"))?;
                seq = _seq;
            }
            if sys.len() > 1 {
                let token_value = &sys[1];
                if !token_value.is_null() {
                    let _token = token_value
                        .as_str()
                        .ok_or(serde::de::Error::custom("sys[1] token is not string"))?;
                    token = Some(_token.to_string());
                }
            }
            if sys.len() > 2 {
                let trace_value = &sys[2];
                if !trace_value.is_null() {
                    let _trace_id = trace_value
                        .as_str()
                        .ok_or(serde::de::Error::custom("sys[2] trace_id is not string"))?;
                    trace_id = Some(_trace_id.to_string());
                }
            }
        }

        Ok(RPCRequest {
            method: method.to_string(),
            params: params.clone(),
            seq,
            token: token,
            trace_id: trace_id,
        })
    }
}

#[derive(Debug, PartialEq)]
pub enum RPCResult {
    Success(Value),
    Failed(String),
}

#[derive(Debug, PartialEq)]
pub struct RPCResponse {
    pub result: RPCResult,

    pub seq: u64,
    pub trace_id: Option<String>,
}

impl RPCResponse {
    pub fn new(result: RPCResult, seq: u64) -> Self {
        RPCResponse {
            result: result,
            seq: seq,
            trace_id: None,
        }
    }

    pub fn create_by_req(result: RPCResult, req: &RPCRequest) -> Self {
        RPCResponse {
            result: result,
            seq: req.seq,
            trace_id: req.trace_id.clone(),
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
                let mut state: <S as Serializer>::SerializeStruct =
                    serializer.serialize_struct("RPCResponse", 1)?;
                state.serialize_field("result", &value)?;

                let mut sys_vec = vec![Value::from(self.seq)];
                if let Some(trace_id) = self.trace_id.as_ref() {
                    sys_vec.push(Value::from(trace_id.clone()));
                }
      
                state.serialize_field("sys", &sys_vec)?;

                state.end()
            }
            RPCResult::Failed(err) => {
                let mut state = serializer.serialize_struct("RPCResponse", 1)?;
                state.serialize_field("error", &err)?;
                let mut sys_vec = vec![Value::from(self.seq)];
                if let Some(trace_id) = self.trace_id.as_ref() {
                    sys_vec.push(Value::from(trace_id.clone()));
                }

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
        let mut seq: u64 = 0;
        let mut trace_id: Option<String> = None;
        if sys.is_some() {
            let sys = sys
                .unwrap()
                .as_array()
                .ok_or(serde::de::Error::custom("sys is not array"))?;

            if sys.len() > 0 {
                let _seq = sys[0]
                    .as_u64()
                    .ok_or(serde::de::Error::custom("sys[0] seq is not u64"))?;
                seq = _seq;
            }

            if sys.len() > 1 {
                let _trace_id = sys[1]
                    .as_str()
                    .ok_or(serde::de::Error::custom("sys[1] trace_id is not string"))?;
                trace_id = Some(_trace_id.to_string());
            }
        }

        if v.get("error").is_some() {
            Ok(RPCResponse {
                result: RPCResult::Failed(v.get("error").unwrap().as_str().unwrap().to_string()),
                seq: seq,
                trace_id: trace_id,
            })
        } else {
            Ok(RPCResponse {
                result: RPCResult::Success(v.get("result").unwrap().clone()),
                seq: seq,
                trace_id: trace_id,
            })
        }
    }
}

//通过RPCContext可以方便跟踪和调试一个长的call chain
//call chain用trace_id来定义
#[derive(Debug, PartialEq,Clone)]
pub struct RPCContext {
    pub seq: u64,
    pub start_time: u64,
    pub token: Option<String>,//jwt session token
    pub trace_id: Option<String>,
    pub from_ip: Option<IpAddr>,
    pub is_rpc:bool,
}

impl Default for RPCContext {
    fn default() -> Self {
        Self {
            seq: 0,
            start_time: 0,
            token: None,
            trace_id: None,
            from_ip: None,
            is_rpc: false,
        }
    }
}

impl RPCContext {
    pub fn from_request(req: &RPCRequest, ip_from: IpAddr) -> Self {
        Self {
            seq: req.seq,
            start_time: buckyos_get_unix_timestamp(),
            token: req.token.clone(),
            trace_id: req.trace_id.clone(),
            from_ip: Some(ip_from),
            is_rpc: true,
        }
    }
}

#[async_trait]
pub trait RPCHandler {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serialize_request_sys_len_1() {
        let mut req = RPCRequest::new("ping", json!({"k":"v"}));
        req.seq = 7;
        let value = serde_json::to_value(req).expect("serialize request");
        assert_eq!(value.get("sys").unwrap(), &json!([7]));
    }

    #[test]
    fn serialize_request_sys_len_2() {
        let mut req = RPCRequest::new("ping", json!({"k":"v"}));
        req.seq = 8;
        req.token = Some("t1".to_string());
        let value = serde_json::to_value(req).expect("serialize request");
        assert_eq!(value.get("sys").unwrap(), &json!([8, "t1"]));
    }

    #[test]
    fn serialize_request_sys_len_3_with_null_token() {
        let mut req = RPCRequest::new("ping", json!({"k":"v"}));
        req.seq = 9;
        req.trace_id = Some("tr1".to_string());
        let value = serde_json::to_value(req).expect("serialize request");
        assert_eq!(value.get("sys").unwrap(), &json!([9, null, "tr1"]));
    }

    #[test]
    fn deserialize_request_sys_len_1() {
        let value = json!({
            "method": "ping",
            "params": {"k": "v"},
            "sys": [7]
        });
        let req: RPCRequest = serde_json::from_value(value).expect("deserialize request");
        assert_eq!(req.seq, 7);
        assert_eq!(req.token, None);
        assert_eq!(req.trace_id, None);
    }

    #[test]
    fn deserialize_request_sys_len_2() {
        let value = json!({
            "method": "ping",
            "params": {"k": "v"},
            "sys": [7, "t1"]
        });
        let req: RPCRequest = serde_json::from_value(value).expect("deserialize request");
        assert_eq!(req.seq, 7);
        assert_eq!(req.token, Some("t1".to_string()));
        assert_eq!(req.trace_id, None);
    }

    #[test]
    fn deserialize_request_sys_len_3_null_token() {
        let value = json!({
            "method": "ping",
            "params": {"k": "v"},
            "sys": [7, null, "tr1"]
        });
        let req: RPCRequest = serde_json::from_value(value).expect("deserialize request");
        assert_eq!(req.seq, 7);
        assert_eq!(req.token, None);
        assert_eq!(req.trace_id, Some("tr1".to_string()));
    }

    #[test]
    fn deserialize_request_sys_len_3_token_trace() {
        let value = json!({
            "method": "ping",
            "params": {"k": "v"},
            "sys": [7, "t1", "tr1"]
        });
        let req: RPCRequest = serde_json::from_value(value).expect("deserialize request");
        assert_eq!(req.seq, 7);
        assert_eq!(req.token, Some("t1".to_string()));
        assert_eq!(req.trace_id, Some("tr1".to_string()));
    }

    #[test]
    fn deserialize_response_success() {
        let value = json!({
            "result": {"ok": true},
            "sys": [11, "tr1"]
        });
        let resp: RPCResponse = serde_json::from_value(value).expect("deserialize response");
        assert_eq!(resp.seq, 11);
        assert_eq!(resp.trace_id, Some("tr1".to_string()));
        assert_eq!(resp.result, RPCResult::Success(json!({"ok": true})));
    }

    #[test]
    fn deserialize_response_error() {
        let value = json!({
            "error": "boom",
            "sys": [12]
        });
        let resp: RPCResponse = serde_json::from_value(value).expect("deserialize response");
        assert_eq!(resp.seq, 12);
        assert_eq!(resp.trace_id, None);
        assert_eq!(resp.result, RPCResult::Failed("boom".to_string()));
    }
}
