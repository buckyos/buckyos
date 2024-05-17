use crate::env::PackageEnv;
use crate::error::PkgSysResult;
use std::path::PathBuf;

pub struct Installer {
    pub env: PackageEnv,
}

impl Installer {
    pub fn new(env: PackageEnv) -> Self {
        Installer { env }
    }

    pub async fn install(&self, pkg_id: &str) -> PkgSysResult<PathBuf> {
        /* 1. 从env中获取pkg_id的信息，如果没有，从index.db中获取
         * 2. 递归安装依赖
         * 3. 安装pkg_id
         */
        unimplemented!()
    }
}

/*
index.json的简化设计：
{
    "deps": {
        "a": {
            "1.0.2": ["b#>2.0", "c#1.0.1"],
            "1.0.1": ["b", "c#<1.0.1"]
        },
        "b": {
            "2.0": ["d#>3.0"],
            "1.0": ["d#<=3.0"]
        },
        "c": {
            "1.0.1": []
        },
        "d": {
            "3.0.1": []
            "3.0.0": []
        }
    },
    ....
}
 */
