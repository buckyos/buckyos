
use crate::{NSCmdHandler, NSCmdRegister};


pub struct HttpNSNodeServer {
    register: NSCmdRegister
}

impl HttpNSNodeServer {
    pub fn new() -> Self {
        Self {
            register: NSCmdRegister::new(),
        }
    }

    // fn register_server(&self, app: &mut sfo_http::http_server::Server<()>) {
    //         app.post("/ns/:cmd_name", move |req, _| {
    //             async move {
    //                 let resp: NSResult<NSCmdResponse> = async move {
    //                     let cmd_name = req.param("cmd_name");
    //                     let request = req.body().await?;
    //                     let response = handler.handle(request.into()).await?;
    //                     Ok(response.into())
    //                 }.await;
    //             }
    //         });
    // }
}
