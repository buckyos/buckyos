use crate::error::{PkgError, PkgResult};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

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

impl FromStr for PackageId {
    type Err = PkgError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Parser::parse(s)
    }
}

impl ToString for PackageId {
    fn to_string(&self) -> String {
        let mut result = self.name.clone();
        if let Some(version) = &self.version {
            result.push_str("#");
            result.push_str(version);
        }
        if let Some(sha256) = &self.sha256 {
            result.push_str("#");
            result.push_str(sha256);
        }
        result
    }
}

impl Parser {
    pub fn parse(pkg_id: &str) -> PkgResult<PackageId> {
        let name;
        let mut version = None;
        let mut sha256 = None;

        let mut parts = pkg_id.split('#');
        if let Some(name_part) = parts.next() {
            name = name_part.to_string();
        } else {
            return Err(PkgError::ParseError(
                pkg_id.to_string(),
                "No name".to_string(),
            ));
        }

        if let Some(version_part) = parts.next() {
            if version_part.starts_with("sha256:") {
                sha256 = Some(version_part.to_string()); //Some(version_part[7..].to_string());
            } else {
                let version_part = version_part.replace(" ", "").replace(",", "");

                if !Self::is_valid_version_expression(&version_part) {
                    return Err(PkgError::ParseError(
                        pkg_id.to_string(),
                        "Invalid version expression".to_string(),
                    ));
                }

                version = Some(version_part);
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

    pub fn get_version_conditions(version_expression: &str) -> PkgResult<Vec<String>> {
        if version_expression == "*" {
            return Ok(vec![version_expression.to_string()]);
        }

        // 如果不是以 '>' 或 '<' 开头，直接判断是否是合法的版本号, 否则就是无效的
        if !version_expression.starts_with('>') && !version_expression.starts_with('<') {
            match Version::parse(version_expression) {
                Ok(_) => return Ok(vec![version_expression.to_string()]),
                Err(err) => {
                    return Err(PkgError::ParseError(
                        version_expression.to_string(),
                        err.to_string(),
                    ));
                }
            }
        }

        // 找到第一个和最后一个 '>' 或 '<' 的位置
        let first_pos = version_expression.find(|c| c == '>' || c == '<');
        let last_pos = version_expression.rfind(|c| c == '>' || c == '<');

        // 如果找到两个不同的位置，进行分割
        if let (Some(first), Some(last)) = (first_pos, last_pos) {
            if first != last {
                let (first_part, last_part) = version_expression.split_at(last);

                if VersionReq::parse(first_part).is_ok() && VersionReq::parse(last_part).is_ok() {
                    return Ok(vec![first_part.to_string(), last_part.to_string()]);
                }
            } else {
                if VersionReq::parse(version_expression).is_ok() {
                    return Ok(vec![version_expression.to_string()]);
                }
            }
        }

        Err(PkgError::ParseError(
            version_expression.to_string(),
            "Invalid version expression".to_string(),
        ))
    }

    pub fn is_valid_version_expression(version_expression: &str) -> bool {
        Self::get_version_conditions(version_expression).is_ok()
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
        assert_eq!(result.sha256, Some("sha256:1234567890".to_string()));

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
