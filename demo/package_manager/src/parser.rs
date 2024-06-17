use crate::env::PackageEnv;
use crate::error::{PackageSystemErrors, PkgSysResult};
use serde::{Deserialize, Serialize};

pub struct Parser {
    pub env: PackageEnv,
}

/*
pkg_id由两部分组成，包名和版本号或者sha256值。例如：
pkg_name
pkg_name#>0.1.4, pkg_name#>=0.1.4
pkg_name#0.1.5
pkg_name#sha256:1234567890
pkg_name#<0.1.6, pkg_name#<=0.1.6
pkg_name#>0.1.4<0.1.6, pkg_name#>0.1.4<=0.1.6, pkg_name#>=0.1.4<0.1.6, pkg_name#>=0.1.4<=0.1.6
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageId {
    pub name: String,
    pub version: Option<String>,
    pub sha256: Option<String>,
}

impl Parser {
    pub fn new(env: PackageEnv) -> Self {
        Parser { env }
    }

    pub fn parse(&self, pkg_id: &str) -> PkgSysResult<PackageId> {
        let name;
        let mut version = None;
        let mut sha256 = None;

        let mut parts = pkg_id.split('#');
        if let Some(name_part) = parts.next() {
            name = name_part.to_string();
        } else {
            return Err(PackageSystemErrors::ParseError(
                pkg_id.to_string(),
                "no name".to_string(),
            ));
        }

        if let Some(version_part) = parts.next() {
            if version_part.starts_with("sha256:") {
                sha256 = Some(version_part[7..].to_string());
                //version = self.get_version_from_sha256(&name, &sha256.as_ref().unwrap())?;
            } else {
                version = Some(version_part.to_string());
                // 这里先不做sha256的查询，等到下载时再查询
            }
        }

        Ok(PackageId {
            name,
            version,
            sha256,
        })
    }
}
