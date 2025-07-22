mod common;
mod named_data_mgr_test;
mod ndn_2_zone_file_api;
mod ndn_2_zone_test_chunk;
mod ndn_2_zone_test_obj;
mod ndn_client_test;

use buckyos_kit::init_logging;
use ndn_2_zone_file_api::*;
use ndn_2_zone_test_chunk::*;
use ndn_2_zone_test_obj::*;

#[tokio::main]
async fn main() {
    unsafe {
        std::env::set_var("RUST_BACKTRACE", "full");
    }

    init_logging("test-ndn", false);

    ndn_2_zone_chunk_ok().await;
    ndn_2_zone_chunk_not_found().await;
    ndn_2_zone_chunk_verify_failed().await;

    ndn_2_zone_object_ok().await;
    ndn_2_zone_object_not_found().await;
    ndn_2_zone_object_verify_failed().await;
    ndn_2_zone_o_link_in_host_innerpath_ok().await;
    ndn_2_zone_o_link_in_host_innerpath_not_found().await;
    ndn_2_zone_o_link_in_host_innerpath_verify_failed().await;
    ndn_2_zone_o_link_innerpath_ok().await;
    ndn_2_zone_o_link_innerpath_not_found().await;
    ndn_2_zone_o_link_innerpath_verify_failed().await;
    ndn_2_zone_r_link_ok().await;
    ndn_2_zone_r_link_not_found().await;
    ndn_2_zone_r_link_verify_failed().await;
    ndn_2_zone_r_link_innerpath_ok().await;
    ndn_2_zone_r_link_innerpath_not_found().await;
    ndn_2_zone_r_link_innerpath_verify_failed().await;

    ndn_2_zone_file_ok().await;
    ndn_2_zone_file_not_found().await;
    ndn_2_zone_file_verify_failed().await;
    ndn_2_zone_o_link_innerpath_file_ok().await;
    ndn_2_zone_o_link_innerpath_file_not_found().await;
    ndn_2_zone_o_link_innerpath_file_verify_failed().await;
    ndn_2_zone_r_link_innerpath_file_ok().await;
    ndn_2_zone_r_link_innerpath_file_not_found().await;
    ndn_2_zone_r_link_innerpath_file_verify_failed().await;
    ndn_2_zone_o_link_innerpath_file_concurrency().await;
    ndn_2_zone_r_link_innerpath_file_concurrency().await;
}
