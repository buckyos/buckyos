use crate::error::*;
use regex::Regex;
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

    pub fn matches(version_condition: &str, version: &str) -> PkgSysResult<bool> {
        /*version_condition可能为以下几种情况：
        *
        >0.1.4  >=0.1.4
        0.1.5
        <0.1.6  <=0.1.6
        >0.1.4<0.1.6    >0.1.4<=0.1.6   >=0.1.4<0.1.6   >=0.1.4<=0.1.6
        >0.1.4 <0.1.6   >0.1.4 <=0.1.6  >=0.1.4 <0.1.6   >=0.1.4 <=0.1.6
        >0.1.4,<0.1.6   >0.1.4,<=0.1.6  >=0.1.4,<0.1.6  >=0.1.4, <=0.1.6
        */
        if version_condition == "*" {
            return Ok(true);
        }

        //如果version_condition不是以> < 开头，则直接比较
        if !version_condition.starts_with('>') && !version_condition.starts_with('<') {
            //如果是以=开头，去掉等号
            let version_condition = if version_condition.starts_with('=') {
                &version_condition[1..]
            } else {
                version_condition
            };
            return Ok(version == version_condition);
        }

        let version = Version::parse(version).map_err(|err| {
            PackageSystemErrors::VersionError(format!(
                "Semver parse error: {}, err:{}",
                version, err
            ))
        })?;

        // 正则表达式匹配条件
        let re = Regex::new(r"([><=]*\d+\.\d+\.\d+)").unwrap();
        let conditions: Vec<&str> = re
            .find_iter(version_condition)
            .map(|mat| mat.as_str())
            .collect();

        // 检查每个条件是否匹配
        for condition in conditions {
            let version_req = VersionReq::parse(condition).map_err(|err| {
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
        assert!(version_util::matches(">1.0.0, <2.0.0", "1.5.0").unwrap());
        assert!(!version_util::matches(">1.0.0, <2.0.0", "2.5.0").unwrap());
        assert!(version_util::matches(">=1.0.0, <=2.0.0", "2.0.0").unwrap());
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
