use async_trait::async_trait;
use buckyos_kit::*;
use jsonwebtoken::{DecodingKey, EncodingKey};
use log::*;
use package_lib::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::hash::Hash;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use crate::run_item::*;
use crate::service_pkg::*;

#[derive(Serialize, Deserialize, Debug)]
pub struct FrameServiceConfig {
    pub target_state: RunItemTargetState,
    //pub name : String, // service name
    pub pkg_id: String,
    pub operations: HashMap<String, RunItemControlOperation>,

    //不支持serizalize
    #[serde(skip)]
    service_pkg: Option<MediaInfo>,
}
