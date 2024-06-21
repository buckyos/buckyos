use crate::error::{PackageSystemErrors, PkgSysResult};
use serde::{Deserialize, Serialize};

pub struct Parser {}

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
    pub fn parse(pkg_id: &str) -> PkgSysResult<PackageId> {
        let name;
        let mut version = None;
        let mut sha256 = None;

        let mut parts = pkg_id.split('#');
        if let Some(name_part) = parts.next() {
            name = name_part.to_string();
        } else {
            return Err(PackageSystemErrors::ParseError(
                pkg_id.to_string(),
                "No name".to_string(),
            ));
        }

        if let Some(version_part) = parts.next() {
            if version_part.starts_with("sha256:") {
                sha256 = Some(version_part[7..].to_string());
            } else {
                let version_part = version_part.replace(" ", "").replace(",", "");
                version = Some(version_part.to_string());
            }
        } else {
            version = Some("*".to_string());
        }

        Ok(PackageId {
            name,
            version,
            sha256,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse() {
        let pkg_id = "a";
        let result = Parser::parse(pkg_id).unwrap();
        assert_eq!(&result.name, "a");
        assert_eq!(result.version, Some("*".to_string()));

        let pkg_id = "a#0.1.0";
        let result = Parser::parse(pkg_id).unwrap();
        assert_eq!(&result.name, "a");
        assert_eq!(result.version, Some("0.1.0".to_string()));

        let pkg_id = "a#sha256:1234567890";
        let result = Parser::parse(pkg_id).unwrap();
        assert_eq!(&result.name, "a");
        assert_eq!(result.sha256, Some("1234567890".to_string()));

        let pkg_id = "a#>0.1.0";
        let result = Parser::parse(pkg_id).unwrap();
        assert_eq!(&result.name, "a");
        assert_eq!(result.version, Some(">0.1.0".to_string()));

        let pkg_id = "a#>0.1.0<0.1.2";
        let result = Parser::parse(pkg_id).unwrap();
        assert_eq!(&result.name, "a");
        assert_eq!(result.version, Some(">0.1.0<0.1.2".to_string()));

        let pkg_id = "a#>0.1.0, <=0.1.2";
        let result = Parser::parse(pkg_id).unwrap();
        assert_eq!(&result.name, "a");
        assert_eq!(result.version, Some(">0.1.0<=0.1.2".to_string()));
    }
}
