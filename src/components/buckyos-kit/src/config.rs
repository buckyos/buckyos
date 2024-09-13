pub struct AppConfig {
    pub app_id : String,
    pub app_name : String,
    pub app_description : String,
    pub vendor_id : String,
    pub pkg_id : String,
    pub user_id: String,
    //dfs mount pint
    pub data_mount_point : String,
    pub cache_mount_point : String,
    //local fs mount point
    pub local_cache_mount_point : String,

    pub max_cpu_num : Option<u32>,
    // 0 - 100
    pub max_cpu_percent : Option<u32>,
    // memory quota in bytes
    pub memory_quota : u64,

    //gateway settings
    pub host_name: Option<String>,
}
