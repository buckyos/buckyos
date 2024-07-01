use crate::error::*;
use crate::parser::Parser;
use log::*;
use semver::{Version, VersionReq};
use std::cmp::Ordering;
use version_compare::{compare as version_compare, Cmp};

pub mod version_util {
    use super::*;

    pub fn compare(a: &str, b: &str) -> PkgSysResult<Ordering> {
        cmp_to_ordering(version_compare(a, b).map_err(|_| {
            PackageSystemErrors::VersionError(format!("Version compare error: {} {}", a, b))
        })?)
    }

    fn cmp_to_ordering(cmp: Cmp) -> PkgSysResult<Ordering> {
        match cmp {
            Cmp::Lt => Ok(Ordering::Less),
            Cmp::Eq => Ok(Ordering::Equal),
            Cmp::Gt => Ok(Ordering::Greater),
            _ => Err(PackageSystemErrors::VersionError(
                "Invalid compare result".to_string(),
            )),
        }
    }

    pub fn matches(version_expression: &str, version: &str) -> PkgSysResult<bool> {
        /*version_condition可能为以下几种情况：
        *
        >0.1.4  >=0.1.4
        0.1.5
        <0.1.6  <=0.1.6
        >0.1.4<0.1.6    >0.1.4<=0.1.6   >=0.1.4<0.1.6   >=0.1.4<=0.1.6
        >0.1.4 <0.1.6   >0.1.4 <=0.1.6  >=0.1.4 <0.1.6   >=0.1.4 <=0.1.6
        >0.1.4,<0.1.6   >0.1.4,<=0.1.6  >=0.1.4,<0.1.6  >=0.1.4, <=0.1.6
        >0.0.1-alpha
        */
        debug!(
            "matches version_expression:{}, version:{}",
            version_expression, version
        );
        if version_expression == "*" {
            return Ok(true);
        }

        let version = Version::parse(version).map_err(|err| {
            PackageSystemErrors::VersionError(format!("Invalid version:{}, err:{}", version, err))
        })?;

        match Parser::get_version_conditions(version_expression) {
            Ok(conditions) => {
                if conditions.len() == 1 {
                    match Version::parse(&conditions[0]) {
                        Ok(version_req) => {
                            return Ok(version == version_req);
                        }
                        Err(_) => match VersionReq::parse(&conditions[0]) {
                            Ok(version_req) => {
                                return Ok(version_req.matches(&version));
                            }
                            Err(err) => {
                                return Err(PackageSystemErrors::ParseError(
                                    version_expression.to_string(),
                                    err.to_string(),
                                ))
                            }
                        },
                    }
                } else {
                    for condition in conditions {
                        let version_req = VersionReq::parse(&condition).map_err(|err| {
                            PackageSystemErrors::VersionError(format!(
                                "VersionReq parse error: {}, err:{}",
                                condition, err
                            ))
                        })?;
                        if !version_req.matches(&version) {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
            }
            Err(err) => Err(err),
        }
    }

    pub fn find_matched_version(
        version_condition: &str,
        versions: &[String],
    ) -> PkgSysResult<String> {
        for version in versions {
            if matches(version_condition, version)? {
                return Ok(version.to_string());
            }
        }

        Err(PackageSystemErrors::VersionError(
            "No matched version found".to_string(),
        ))
    }
}

pub use version_util::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn test_compare() {
        assert_eq!(
            version_util::compare("1.0.0", "1.0.1").unwrap(),
            Ordering::Less
        );
        assert_eq!(
            version_util::compare("1.0.1", "1.0.0").unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            version_util::compare("1.0.0", "1.0.0").unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn test_matches() {
        assert!(version_util::matches(">1.0.0", "1.0.1").unwrap());
        assert!(!version_util::matches(">1.0.1", "1.0.0").unwrap());
        assert!(version_util::matches(">=1.0.0", "1.0.0").unwrap());
        assert!(version_util::matches("<1.0.1", "1.0.0").unwrap());
        assert!(version_util::matches("<=1.0.0", "1.0.0").unwrap());
        assert!(version_util::matches(">1.0.0<2.0.0", "1.5.0").unwrap());
        assert!(!version_util::matches(">1.0.0<2.0.0", "2.5.0").unwrap());
        assert!(version_util::matches(">=1.0.0<=2.0.0", "2.0.0").unwrap());
        assert!(version_util::matches("*", "2.0.0").unwrap());
        assert!(version_util::matches("1.0.1", "1.0.1").unwrap());
        assert!(!version_util::matches("1.0.1", "1.0.2").unwrap());
    }

    #[test]
    fn test_find_matched_version() {
        let versions = vec![
            "2.0.0".to_string(),
            "1.1.0".to_string(),
            "1.0.1".to_string(),
            "1.0.0".to_string(),
        ];

        assert_eq!(
            version_util::find_matched_version("1.0.0", &versions).unwrap(),
            "1.0.0"
        );

        assert_eq!(
            version_util::find_matched_version(">1.0.0<1.1.0", &versions).unwrap(),
            "1.0.1"
        );
        assert_eq!(
            version_util::find_matched_version(">=1.0.0<1.0.1", &versions).unwrap(),
            "1.0.0"
        );
        assert_eq!(
            version_util::find_matched_version(">=1.0.0<1.0.1", &versions).unwrap(),
            "1.0.0"
        );
        assert_eq!(
            version_util::find_matched_version(">=1.0.0<1.0.1", &versions).unwrap(),
            "1.0.0"
        );
        assert_eq!(
            version_util::find_matched_version(">1.0.0<=1.0.1", &versions).unwrap(),
            "1.0.1"
        );
        assert_eq!(
            version_util::find_matched_version("<2.0.0", &versions).unwrap(),
            "1.1.0"
        );
        assert_eq!(
            version_util::find_matched_version("<=1.1.0", &versions).unwrap(),
            "1.1.0"
        );

        assert!(version_util::find_matched_version(">2.0.0", &versions).is_err());
    }
}
