use crate::error::{PkgError, PkgResult};
use semver::*;
use serde::{Deserialize, Serialize};
use version_compare::Cmp;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub enum VersionExpType {
    None,
    Req(VersionReq),
    Version(Version),
}

impl ToString for VersionExpType {
    fn to_string(&self) -> String {
        match self {
            VersionExpType::Req(req) => req.to_string(),
            VersionExpType::Version(version) => version.to_string(),
            VersionExpType::None => "".to_string(),
        }
    }
}

impl FromStr for VersionExpType {
    type Err = PkgError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let version = Version::parse(s);
        if version.is_ok() {
            return Ok(VersionExpType::Version(version.unwrap()));
        }

        let req = VersionReq::parse(s);
        if req.is_ok() {
            return Ok(VersionExpType::Req(req.unwrap()));
        }

        return Ok(VersionExpType::None);
    }
}

#[derive(Debug, Clone)]
pub struct VersionExp {
    pub tag: Option<String>,
    pub version_exp: VersionExpType,
}

impl Default for VersionExp {
    fn default() -> Self {
        VersionExp { tag: None, version_exp: VersionExpType::None }
    }
}

impl FromStr for VersionExp {
    type Err = PkgError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(":").collect::<Vec<&str>>();
        match parts.len() {
            1 => {
                let version_exp = VersionExpType::from_str(parts[0])?;
                return Ok(VersionExp { tag: None, version_exp });
            }
            2 => {
                let tag = parts[1].to_string();
                let version_exp = VersionExpType::from_str(parts[0])?;
                return Ok(VersionExp { tag: Some(tag), version_exp });
            }
            _ => {
                return Err(PkgError::ParseError(s.to_string(), "Invalid version expression".to_string()));
            }
        }
    }
}

impl ToString for VersionExp {
    fn to_string(&self) -> String {
        if let Some(tag) = &self.tag {
            format!("{}:{}", self.version_exp.to_string(), tag)
        } else {
            self.version_exp.to_string()
        }
    }
}
 
impl VersionExp {
    pub fn is_version(&self) -> bool {
        matches!(self.version_exp, VersionExpType::Version(_))
    }

    pub fn to_range_int(&self) -> PkgResult<(u64, u64)> {
        match &self.version_exp {
            VersionExpType::Req(req) => {
                match req.comparators.len() {
                    1 => {
                        let comparator = &req.comparators[0];
                        match comparator.op {
                            Op::Greater | Op::GreaterEq => {
                                let min = Self::comparator_to_int(comparator)?;
                                let max = i64::MAX;
                                Ok((min as u64, max as u64))
                            }
                            Op::Less | Op::LessEq => {
                                let min = i64::MIN;
                                let max = Self::comparator_to_int(comparator)?;
                                Ok((min as u64, max as u64))
                            }
                            _ => {
                                return Err(PkgError::ParseError(self.to_string(), "VersionExp can not be converted to range int".to_string()));
                            }
                        }
                    },
                    2 => {
                        let comparator1 = &req.comparators[0];
                        let comparator2 = &req.comparators[1];
                        let min = Self::comparator_to_int(comparator1)?;
                        let max = Self::comparator_to_int(comparator2)?;
                        if min > max {
                            return Ok((max, min));
                        }
                        Ok((min, max))

                    },
                    _ => {
                        return Err(PkgError::ParseError(self.to_string(), "VersionExp can not be converted to range int".to_string()));
                    }
                }
            },
            _ => {
                return Err(PkgError::ParseError(self.to_string(), "VersionExp can not be converted to range int".to_string()));
            }
        }
    }
    
    pub fn comparator_to_int(comparator: &Comparator) -> PkgResult<u64> {
        let major = comparator.major;
        let minor = comparator.minor.unwrap_or(0);
        let patch = comparator.patch.unwrap_or(0);
        let build_str = comparator.pre.to_string();
        let digits_only = build_str.trim_start_matches(|c: char| !c.is_digit(10));
        let build = digits_only.parse::<u64>().unwrap_or(0);
        
        let version_int = (major as u64) << 56 | (minor as u64) << 40 | (patch as u64) << 24 | build as u64;   
        Ok(version_int)
    }

    // 将版本号转换为整数表示
    pub fn version_to_int(version: &str) -> PkgResult<u64> {
        // 处理semver格式，先移除预发布版本和构建元数据部分
        let build_pos = version.find(|c| c == '-' || c == '+'); 
        let version_core = if let Some(pos) = build_pos {
            &version[0..pos]
        } else {
            version
        };
        let mut parts: Vec<&str> = version_core.split('.').collect();
        
        // 基本格式检查
        if parts.len() < 1 || parts.len() > 4 {
            return Err(PkgError::VersionError(format!("无效的版本格式: {}", version)));
        }

        if parts.len() == 3 {
            if build_pos.is_some() {
                let build_str = &version[build_pos.unwrap()..];
                parts.push(build_str);
            }
        }
        
        // 解析各部分
        let major = parts.get(0).and_then(|v| {
            // 忽略第一个数字前的其它字符
            let digits_only = v.trim_start_matches(|c: char| !c.is_digit(10));
            digits_only.parse::<u64>().ok()
        }).unwrap_or(0);
        if major > 0xff {
            return Err(PkgError::VersionError(format!("主版本号超出范围: {}", version)));
        }

        let minor = parts.get(1).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        if minor > 0xffff {
            return Err(PkgError::VersionError(format!("次版本号超出范围: {}", version)));
        }
        
        let patch = parts.get(2).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        if patch > 0xffff {
            return Err(PkgError::VersionError(format!("补丁版本号超出范围: {}", version)));
        }   
        
        let build =  parts.get(3).and_then(|v| {
            // 忽略第一个数字前的其它字符
            let digits_only = v.trim_start_matches(|c: char| !c.is_digit(10));
            digits_only.parse::<u64>().ok()
        }).unwrap_or(0);
        if build > 0xffffff {
            return Err(PkgError::VersionError(format!("构建号超出范围: {}", version)));
        }
        //0xff , 0xffff, 0xffff, 0xffffff ,build号用24位，支持 15-12-25 这样的6位日期
        let version_int = (major as u64) << 56 | (minor as u64) << 40 | (patch as u64) << 24 | build as u64;
        
        Ok(version_int)
    }


    pub fn compare_versions(v1: &str, v2: &str) -> std::cmp::Ordering {
        match (semver::Version::parse(v1), semver::Version::parse(v2)) {
            (Ok(v1), Ok(v2)) => v1.cmp(&v2),
            // 处理非标准版本格式的情况
            _ => {
                // 自定义比较逻辑，使用我们的整数表示进行比较
                match (Self::version_to_int(v1), Self::version_to_int(v2)) {
                    (Ok(v1_int), Ok(v2_int)) => v1_int.cmp(&v2_int),
                    // 如果转换失败，则按字符串比较
                    _ => v1.cmp(v2)
                }
            }
        }
    }
}

/*
pkg_id由两部分组成，包名和版本号或者sha256值。例如：
pkg_name : pkg_name在整个Index中是唯一的，一般默认用pkg_author.module_name 来表示。只用Pkg_name使用的是默认版本
pkd_name#0.1.5 : 指定版本
pkg_name#:latest : 指定latest版本
pkg_name#>0.1.4,<=0.1.6:stable : 指定范围版本里的,有stable tag的版本
pkg_name#$objid : 指定一个精确版本
pkg_name#0.1.5#$objid : 语义更强的指定一个精确版本，在加载的时候会对版本号进行验证
 */
#[derive(Debug, Clone)]
pub struct PackageId {
    pub name: String,
    pub version_exp: Option<VersionExp>,
    pub objid: Option<String>,
}


impl FromStr for PackageId {
    type Err = PkgError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        PackageId::parse(s)
    }
}

impl ToString for PackageId {
    fn to_string(&self) -> String {
        let mut result = self.name.clone();
        if let Some(version) = &self.version_exp {
            result.push_str("#");
            result.push_str(&version.to_string());
        }
        if let Some(objid) = &self.objid {
            result.push_str("#");
            result.push_str(objid);
        }
        result
    }
}

impl PackageId {
    //todo
    pub fn get_author(full_name: &str) -> Option<String> {
        if let Some(pos) = full_name.find('.') {
            let author = &full_name[0..pos];
            return Some(author.to_string());
        } else {
            return None;
        }
    }

    pub fn parse(pkg_id: &str) -> PkgResult<PackageId> {
        let parts = pkg_id.split('#').collect::<Vec<&str>>();
        match parts.len() {
            1 => {
                return Ok(PackageId {
                    name: parts[0].to_string(),
                    version_exp: None,
                    objid: None,
                });
            },
            2 => {
                let name = parts[0].to_string();
                let version_part = parts[1].to_string();
                if version_part.contains(".") || version_part.contains(":") || version_part.contains("*") {
                    let version_exp = VersionExp::from_str(&version_part)?;
                    return Ok(PackageId {
                        name: name,
                        version_exp: Some(version_exp),
                        objid: None,
                    });
                } else {
                    return Ok(PackageId {
                        name: name,
                        version_exp: None,
                        objid: Some(version_part),
                    });
                }
            },
            3=>{
                let name = parts[0].to_string();
                let version_part = parts[1].to_string();
                let objid_part = parts[2].to_string();
                let version_exp = VersionExp::from_str(&version_part)?;
                return Ok(PackageId {
                    name: name,
                    version_exp: Some(version_exp),
                    objid: Some(objid_part),
                });
            }
            _ => {
                return Err(PkgError::ParseError(pkg_id.to_string(), "Invalid package id".to_string()));
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;

    #[test]
    fn test_parse() {
        let pkg_id = "a";
        let result = PackageId::parse(pkg_id).unwrap();
        assert_eq!(&result.name, "a");
        let pkg_id2 = result.to_string();
        assert_eq!(pkg_id, pkg_id2);

        let pkg_id = "a#0.1.0:stable";
        let result = PackageId::parse(pkg_id).unwrap();
        assert_eq!(&result.name, "a");
        assert_eq!(result.version_exp.as_ref().unwrap().to_string(), "0.1.0:stable".to_string());
        assert_eq!(result.version_exp.as_ref().unwrap().tag,Some("stable".to_string()));
        let pkg_id2 = result.to_string();
        assert_eq!(pkg_id, pkg_id2);

        let pkg_id = "a#1234567890";
        let result = PackageId::parse(pkg_id).unwrap();
        assert_eq!(&result.name, "a");
        assert_eq!(result.objid, Some("1234567890".to_string()));
        let pkg_id2 = result.to_string();
        assert_eq!(pkg_id, pkg_id2);

        let pkg_id = "a#>0.1.0";
        let result = PackageId::parse(pkg_id).unwrap();
        assert_eq!(&result.name, "a");
        assert_eq!(result.version_exp.as_ref().unwrap().to_string(), ">0.1.0".to_string());
        let pkg_id2 = result.to_string();
        assert_eq!(pkg_id, pkg_id2);

        let pkg_id = "a#>0.1.0, <0.1.2:stable";
        let result = PackageId::parse(pkg_id).unwrap();
        assert_eq!(&result.name, "a");
        //println!("result.version_exp: {:?}", result.version_exp.as_ref().unwrap().to_string());
        assert_eq!(result.version_exp.as_ref().unwrap().to_string(), ">0.1.0, <0.1.2:stable".to_string());
        let pkg_id2 = result.to_string();
        assert_eq!(pkg_id, pkg_id2);
    }

    #[test]
    fn test_version_to_int() -> PkgResult<()> {
        // 测试版本号转整数
        let test_cases = vec![
            ("1", 0x01_0000_0000_000000),
            ("1.0", 0x01_0000_0000_000000),
            ("1.2", 0x01_0002_0000_000000),
            ("1.2.3", 0x01_0002_0003_000000),
            ("1.2.3.4", 0x01_0002_0003_000004),
            ("10.20.30.40", 0x0A_0014_001E_000028),
            ("0.0.0.0", 0x00_0000_0000_000000),
            ("1.0.3-build250326", 0x01_0000_0003_03d1d6),
            ("1.0.0-alpha_123", 0x01_0000_0000_00007b),
            (">1.0.3-build250326", 0x01_0000_0003_03d1d6),
        ];

        for (version, expected) in &test_cases {
            let result =  VersionExp::version_to_int(version)?;
            assert_eq!(result, *expected, "版本 {} 转换为整数应该是 {:#X}, 但得到了 {:#X}", version, expected, result);
        }

        Ok(())
    }

    #[test]
    fn test_comparator_to_int() -> PkgResult<()> {
        let comparator = VersionExp::comparator_to_int(&Comparator::parse(">1.0.3-build250326").unwrap()).unwrap();
        assert_eq!(comparator, 0x01_0000_0003_03d1d6);

        let package_id = PackageId::parse("a#>1.0.3-build250326, <=1.0.4-build250426").unwrap();
        let range = package_id.version_exp.as_ref().unwrap().to_range_int().unwrap();
        assert_eq!(range, (0x01_0000_0003_03d1d6, 0x01_0000_0004_03d23a));
        Ok(())
    }

    #[test]
    fn test_version_comparison() -> PkgResult<()> {
        // 测试标准semver格式的版本比较
        let semver_test_cases = vec![
            ("1.0.0", "1.0.0", Ordering::Equal),
            ("1.0.0", "1.0.1", Ordering::Less),
            ("1.0.1", "1.0.0", Ordering::Greater),
            ("1.0.0", "1.1.0", Ordering::Less),
            ("1.1.0", "1.0.0", Ordering::Greater),
            ("1.0.0", "2.0.0", Ordering::Less),
            ("2.0.0", "1.0.0", Ordering::Greater),
            ("1.0.0-alpha", "1.0.0", Ordering::Less),
            ("1.0.0", "1.0.0-alpha", Ordering::Greater),
            ("1.0.0-alpha", "1.0.0-beta", Ordering::Less),
            ("1.0.0-beta", "1.0.0-alpha", Ordering::Greater),
            ("1.0.0-beta", "1.0.0-alpha+323ad", Ordering::Greater),
        ];

        for (v1, v2, expected) in semver_test_cases {
            let result = VersionExp::compare_versions(v1, v2);
            assert_eq!(result, expected, "比较 {} 和 {} 应该得到 {:?}, 但得到了 {:?}", v1, v2, expected, result);
        }

        // 测试非标准格式的版本比较（使用我们的自定义逻辑）
        let custom_test_cases = vec![
            ("1", "1", Ordering::Equal),
            ("1", "1.0", Ordering::Equal),
            ("1.0", "1.0.0", Ordering::Equal),
            ("1", "2", Ordering::Less),
            ("2", "1", Ordering::Greater),
            ("1.2", "1.3", Ordering::Less),
            ("1.3", "1.2", Ordering::Greater),
            ("1.2.3", "1.2.4", Ordering::Less),
            ("1.2.4", "1.2.3", Ordering::Greater),
            ("1.2.3.4", "1.2.3.5", Ordering::Less),
            ("1.2.3.5", "1.2.3.4", Ordering::Greater),
            ("1.2.3", "1.2.3.0", Ordering::Equal),
            ("1.2.0", "1.2", Ordering::Equal),
            ("1.0.0", "1", Ordering::Equal),
        ];

        for (v1, v2, expected) in custom_test_cases {
            let result = VersionExp::compare_versions(v1, v2);
            assert_eq!(result, expected, "比较 {} 和 {} 应该得到 {:?}, 但得到了 {:?}", v1, v2, expected, result);
        }

        Ok(())
    }
}
