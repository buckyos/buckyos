mod kv_provider;
//mod etcd_provider;
//mod rocksdb_provider;
mod sled_provider;

use warp::Filter;
use std::sync::Arc;

use kv_provider::*;
//use etcd_provider::*;
//use rocksdb_provider::*;
use sled_provider::*; 

#[tokio::main]
async fn main() {
    // Select the rear end storage, here you can switch different implementation
    let store: Arc<dyn KVStoreProvider> = Arc::new(
        //EtcdStore::new(&["http://127.0.0.1:2379"]).await.unwrap()
        // RocksDBStore::new("./system_config.rsdb").unwrap()
        SledStore::new("sled_db").unwrap()
    );

    let store_filter = warp::any().map(move || Arc::clone(&store));

    // GET /system_config/<key>
    let get_route = warp::path!("system_config" / String)
        .and(warp::get())
        .and(store_filter.clone())
        .and_then(handle_get);

    // POST /system_config/<key> { "value": "<value>" }
    let set_route = warp::path!("system_config" / String)
        .and(warp::post())
        .and(warp::body::json())
        .and(store_filter.clone())
        .and_then(handle_set);

    let routes = get_route.or(set_route);

    warp::serve(routes).run(([0, 0, 0, 0], 3030)).await;
}

async fn handle_get(key: String, store: Arc<dyn KVStoreProvider>) -> Result<impl warp::Reply, warp::Rejection> {
    //TODO:ACL control here

    match store.get(key).await {
        Ok(Some(value)) => Ok(warp::reply::json(&serde_json::json!({ "value": value }))),
        Ok(None) => Ok(warp::reply::json(&serde_json::json!({ "resp": "Key not found" }))),
        Err(_) => Ok(warp::reply::json(&serde_json::json!({ "resp": "Internal error" }))),
    }
}

async fn handle_set(key: String, body: serde_json::Value, store: Arc<dyn KVStoreProvider>) -> Result<impl warp::Reply, warp::Rejection> {
    //TODO:ACL control here

    if let Some(value) = body.get("value").and_then(|v| v.as_str()) {
        if store.set(key, value.to_string()).await.is_ok() {
            Ok(warp::reply::with_status("OK", warp::http::StatusCode::OK))
        } else {
            Ok(warp::reply::with_status("Internal error", warp::http::StatusCode::INTERNAL_SERVER_ERROR))
        }
    } else {
        Ok(warp::reply::with_status("Bad request", warp::http::StatusCode::BAD_REQUEST))
    }
}
