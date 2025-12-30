// NOTE: warp dependency has been removed, this code is commented out
// use wrap::Filter;
// use crate::{KRaft, network::request::RaftResponse};
// use openraft::Raft;
// use super::request::{RaftRequest, RaftRequestType};
// use crate::{KNode, KNodeId, KTypeConfig, KRaftRef};

// pub struct KNetworkServer {
//     addr: String,
//     raft: KRaftRef,
// }

// impl KNetworkServer {
//     pub fn new(addr: String, raft: KRaftRef) -> Self {
//         Self { addr, raft }
//     }

//     async fn on_append_entries(req: RaftRequest) -> RaftResponse {
//         // Handle the AppendEntries request
//         println!("Received AppendEntries request: {:?}", req);
//         warp::reply::json(&"AppendEntries response")
//     }

//     pub fn run(&self) {
//         let append_entries = warp::path(RaftRequestType::AppendEntries.as_str())
//             .and(warp::post())
//             .and(warp::body::bytes())
//             .map(|body: bytes::Bytes| {
//                 // Deserialize the request
//                 match RaftRequest::deserialize(&body) {
//                     Ok(req) => {
//                         let ret = match self.
//                     }
//                     Err(e) => {
//                         eprintln!("Failed to deserialize AppendEntries request: {}", e);
//                         warp::reply::json(&"Error")
//                     }
//                 }
//                 // Handle the request (this is just a placeholder)
//                 println!("Received AppendEntries request: {:?}", req);
//                 // Respond with a placeholder response
//                 warp::reply::json(&"AppendEntries response")
//             });

//         let install_snapshot = warp::path("klog")
//             .and(warp::path("install-snapshot"))
//             .and(warp::post())
//             .and(warp::body::bytes())
//             .map(|body: bytes::Bytes| {
//                 let req = RaftRequest::deserialize(&body).unwrap();
//                 println!("Received InstallSnapshot request: {:?}", req);
//                 warp::reply::json(&"InstallSnapshot response")
//             });

//         let vote = warp::path("klog")
//             .and(warp::path("vote"))
//             .and(warp::post())
//             .and(warp::body::bytes())
//             .map(|body: bytes::Bytes| {
//                 let req = RaftRequest::deserialize(&body).unwrap();
//                 println!("Received Vote request: {:?}", req);
//                 warp::reply::json(&"Vote response")
//             });

//         let routes = append_entries.or(install_snapshot).or(vote);

//         let addr = self.addr.parse().unwrap();
//         println!("Starting server at {}", self.addr);
//         warp::serve(routes).run(addr);
//     }
// }