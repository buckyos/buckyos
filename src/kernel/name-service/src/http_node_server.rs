use sfo_http::http_server::{Request, Response, StatusCode};
use sfo_http::http_server::http::mime::JSON;
use sfo_serde_result::SerdeResult;
use crate::{NSCmdRegisterRef, NSCmdRequest, NSCmdResponse, NSErrorCode, NSResult};
use crate::error::{ns_err};


pub struct HttpNSNodeServer {
    register: NSCmdRegisterRef
}

impl HttpNSNodeServer {
    pub fn new(register: NSCmdRegisterRef) -> Self {
        Self {
            register,
        }
    }

    fn register_server(&self, app: &mut sfo_http::http_server::Server<()>) {
        let register = self.register.clone();
        app.at("/ns/:cmd_name").post(move |mut req: Request<()>| {
            let register = register.clone();
            async move {
                let resp: NSResult<NSCmdResponse> = async move {
                    let cmd_name = req.param("cmd_name")
                        .map_err(|e| {
                            ns_err!(NSErrorCode::InvalidParam, "Failed to get param: {}", e)
                        })?;
                    if let Some(handler) = register.get_cmd_handler(cmd_name) {
                        let body = req.body_bytes().await
                            .map_err(|e| {
                                ns_err!(NSErrorCode::InvalidParam, "Failed to get body: {}", e)
                            })?;
                        let request = NSCmdRequest::from(body);
                        Ok(handler.handle(request).await)
                    } else {
                        Err(ns_err!(NSErrorCode::Forbid, "Cmd {} not found", cmd_name))
                    }
                }.await;
                let mut http_resp = Response::new(StatusCode::Ok);
                http_resp.set_content_type(JSON);
                if let Err(e) = resp {
                    http_resp.set_body(serde_json::to_string(&SerdeResult::from(NSResult::<()>::Err(e))).unwrap());
                } else {
                    http_resp.set_body(<NSCmdResponse as Into<Vec<u8>>>::into(resp.unwrap()));
                }
                Ok(http_resp)
            }
        });
    }
}
