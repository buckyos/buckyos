use crate::error::*;
use async_trait::async_trait;
use log::*;
use std::path::PathBuf;

pub struct Installer {
    env_dir: PathBuf,
}

impl Installer {
    pub fn new(env_dir: PathBuf) -> Self {
        Installer { env_dir }
    }

    pub async fn install_pkg(&self, pkg_id_str: &str) -> PkgResult<()> {
        debug!("install pkg:{}", pkg_id_str);
        Ok(())
    }
}
