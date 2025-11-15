mod server;
mod storage;

#[macro_use]
extern crate log;

use crate::server::LogHttpServer;

#[tokio::main]
async fn main() {
    let addr = "0:0:0:0:8089";
}
