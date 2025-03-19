use buckyos_kit::get_buckyos_system_etc_dir;
use jsonwebtoken::EncodingKey;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use name_lib::{decode_json_from_jwt_with_default_pk, DeviceConfig};

