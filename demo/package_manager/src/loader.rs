use crate::env::PackageEnv;
use crate::parser::*;
use std::path::PathBuf;

pub struct Loader {
    pub env: PackageEnv,
}

#[derive(Debug)]
pub enum MediaType {
    Dir,
    File,
}

/* MediaInfo是一个包的元信息
  包括pkg_id，
  类型（dir or file）
  完整路径
*/
#[derive(Debug)]
pub struct MediaInfo {
    pub pkg_id: PackageId,
    pub full_path: PathBuf,
    pub media_type: MediaType,
}

impl Loader {
    pub fn new(env: PackageEnv) -> Self {
        Loader { env }
    }

    // pub async fn load(&self, pkg_id_str: &str) -> PkgSysResult<MediaInfo> {
    //     let parser = Parser::new(self.env.clone());

    //     let pkg_id = parser.parse(pkg_id_str)?;

    //     if let Some(version) = &pkg_id.version {
    //         //如果version不是以>=,<=,>,<开头，就是精确版本号
    //         if !version.starts_with(">") && !version.starts_with("<") {
    //             let full_path = self
    //                 .env
    //                 .work_dir
    //                 .join(format!("{}#{}", pkg_id.name, version));
    //             info!("get full path for {} -> {:?}", pkg_id_str, full_path);

    //             return self.load_with_full_path(&pkg_id, &full_path);
    //         }
    //     }

    //     //如果有精确的sha256值，也可以拼接
    //     if let Some(sha256) = &pkg_id.sha256 {
    //         let full_path = self
    //             .env
    //             .work_dir
    //             .join(format!("{}#{}", pkg_id.name, sha256));
    //         info!("get full path for {} -> {:?}", pkg_id_str, full_path);

    //         if let Ok(media_info) = self.load_with_full_path(&pkg_id, &full_path) {
    //             return Ok(media_info);
    //         }
    //     }

    //     self.load_with_version_expression(&pkg_id)
    // }

    // fn load_with_full_path(
    //     &self,
    //     pkg_id: &PackageId,
    //     full_path: &PathBuf,
    // ) -> PkgSysResult<MediaInfo> {
    //     if full_path.exists() {
    //         let media_type = if full_path.is_dir() {
    //             MediaType::Dir
    //         } else {
    //             MediaType::File
    //         };

    //         Ok(MediaInfo {
    //             pkg_id: pkg_id.clone(),
    //             full_path: full_path.clone(),
    //             media_type,
    //         })
    //     } else {
    //         Err(PackageSystemErrors::LoadError(
    //             full_path.to_str().unwrap().to_string(),
    //             "not found".to_string(),
    //         ))
    //     }
    // }

    // fn load_with_version_expression(&self, pkg_id: &PackageId) -> PkgSysResult<MediaInfo> {
    //     let mut min_version = None;
    //     let mut max_version = None;
    //     let mut inclusive_min = false;
    //     let mut inclusive_max = false;

    //     if let Some(version) = &pkg_id.version {
    //         if !version.starts_with(">") && !version.starts_with("<") {
    //             return Err(PackageSystemErrors::LoadError(
    //                 pkg_id.name.clone(),
    //                 "Invalid version expression".to_string(),
    //             ));
    //         }

    //         // 使用正则表达式来匹配版本号和操作符， 一般是类似>1.0.2 或者 >1.0.2<1.0.5这样的版本表达式
    //         let re = Regex::new(r"(>=|<=|>|<)(\d+\.\d+\.\d+)").unwrap();

    //         for cap in re.captures_iter(version) {
    //             match &cap[1] {
    //                 ">=" => {
    //                     min_version = Some(cap[2].to_string());
    //                     inclusive_min = true;
    //                 }
    //                 ">" => {
    //                     min_version = Some(cap[2].to_string());
    //                 }
    //                 "<=" => {
    //                     max_version = Some(cap[2].to_string());
    //                     inclusive_max = true;
    //                 }
    //                 "<" => {
    //                     max_version = Some(cap[2].to_string());
    //                 }
    //                 _ => {}
    //             }
    //         }
    //     }

    //     info!("load_with_version_expression: min_version:{:?}, inclusive_min:{}, max_version:{:?}, inclusive_max:{}",
    //     min_version, inclusive_min, max_version, inclusive_max);

    //     // 找到符合条件的版本
    //     let matching_version = self.find_matching_version(
    //         &pkg_id,
    //         min_version,
    //         max_version,
    //         inclusive_min,
    //         inclusive_max,
    //     )?;

    //     let full_path = self
    //         .work_dir
    //         .join(format!("{}#{}", pkg_id.name, matching_version));

    //     self.load_with_full_path(&pkg_id, &full_path)
    // }

    // fn find_matching_version(
    //     &self,
    //     pkg_id: &PackageId,
    //     min_version: Option<String>,
    //     max_version: Option<String>,
    //     inclusive_min: bool,
    //     inclusive_max: bool,
    // ) -> PkgSysResult<String> {
    //     let pkg_name = &pkg_id.name;
    //     let version_expression = &pkg_id.version;
    //     let mut pkg_full_name = String::from(pkg_name);
    //     if version_expression.is_some() {
    //         pkg_full_name += "#";
    //         pkg_full_name += version_expression.as_ref().unwrap();
    //     }
    //     //TODO 先查询index_db
    //     let index_db_path = self.work_dir.join("index_db");
    //     if index_db_path.exists() {
    //         //TODO 从index_db中查询
    //         todo!("query index_db");
    //     }

    //     //TODO 查询lock文件

    //     //遍历目录，找到所有包名匹配的目录
    //     let mut matching_versions: Vec<String> = Vec::new();
    //     let pkgs_dir = self.work_dir.clone();
    //     if pkgs_dir.exists() {
    //         for entry in pkgs_dir.read_dir().unwrap() {
    //             if let Ok(entry) = entry {
    //                 if entry.path().is_dir() {
    //                     let file_name = entry.file_name().into_string().unwrap();
    //                     if !file_name.starts_with(pkg_name) {
    //                         continue;
    //                     }
    //                     //以#分割，取第最后一部分作为版本号
    //                     let parts: Vec<&str> = file_name.split("#").collect();
    //                     let version = parts[parts.len() - 1];
    //                     if let Some(min_version) = &min_version {
    //                         let compare_ret: Cmp =
    //                             compare(&version, min_version).map_err(|_err| {
    //                                 PackageSystemErrors::LoadError(
    //                                     pkg_full_name.clone(),
    //                                     "Compare version error".to_string(),
    //                                 )
    //                             })?;
    //                         if compare_ret == Cmp::Lt {
    //                             continue;
    //                         }
    //                         if compare_ret == Cmp::Eq && !inclusive_min {
    //                             continue;
    //                         }
    //                     }

    //                     if let Some(max_version) = &max_version {
    //                         let compare_ret: Cmp =
    //                             compare(&version, max_version).map_err(|_err| {
    //                                 PackageSystemErrors::LoadError(
    //                                     pkg_full_name.clone(),
    //                                     "Compare version error".to_string(),
    //                                 )
    //                             })?;
    //                         if compare_ret == Cmp::Gt {
    //                             continue;
    //                         }
    //                         if compare_ret == Cmp::Eq && !inclusive_max {
    //                             continue;
    //                         }
    //                     }

    //                     matching_versions.push(version.to_owned());
    //                 }
    //             }
    //         }
    //         //在matching_versions中选择版本最高的
    //         if matching_versions.is_empty() {
    //             return Err(PackageSystemErrors::LoadError(
    //                 pkg_full_name.clone(),
    //                 "No matching version found".to_string(),
    //             ));
    //         } else {
    //             let mut result_version = matching_versions[0].clone();
    //             for version in matching_versions {
    //                 if compare(&version, &result_version).unwrap() == Cmp::Gt {
    //                     result_version = version;
    //                 }
    //             }
    //             return Ok(result_version.to_owned());
    //         }
    //     } else {
    //         return Err(PackageSystemErrors::LoadError(
    //             pkg_full_name,
    //             "Not found".to_string(),
    //         ));
    //     }
    // }
}
