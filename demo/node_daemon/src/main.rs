use log::*;
use simplelog::*;
use std::fs::File;

fn init_log_config() {
    // 创建一个日志配置对象
    let config = ConfigBuilder::new().build();

    // 初始化日志器
    CombinedLogger::init(vec![
        // 将日志输出到标准输出，例如终端
        TermLogger::new(LevelFilter::Info, config.clone(), TerminalMode::Mixed, ColorChoice::Auto),
        // 同时将日志输出到文件
        WriteLogger::new(LevelFilter::Info, config, File::create("node_daemon.log").unwrap())
    ]).unwrap();
}

fn load_identity_config() -> Result<(), Box<dyn std::error::Error>>{

} 

fn looking_zone_config() -> Result<(), Box<dyn std::error::Error>>{

}


async fn check_etcd_by_zone_config(config) -> Result<(), Box<dyn std::error::Error>>{

}

fn execute_docker(docker_config)   -> Result<(), Box<dyn std::error::Error>>{
    for docker_instance in docker_config {
        //尝试启动/停止镜像
        //启动镜像前，需要通知zone内的docker repo先更新必要的镜像。该过程和docekr repo的实现是解耦合的，后续可以用
    }
}

fn execute_service(service_config)  -> Result<(), Box<dyn std::error::Error>>{
    for service_instance in service_config {
        //service一定不跑在docker里
        //尝试启动/停止/更新服务
        
    }
}


fn node_daemon_main_loop() -> Result<(), Box<dyn std::error::Error>>{
    etcd_client = create_etcd_client()
    etcd_client.refresh_config()
    system_config.init()
    cmd_config = system_config.get("")
    execute_cmd(cmd_config) //一般是执行运维命令，类似系统备份和恢复
    service_config = system_config.get("")
    execute_service(service_config)
    vm_config = system_config.get("")
    execute_vm(vm_config)
    docker_config = system_config.get("")
    execute_docker(docker_config)
}


fn main() {
    init_log_config();
    info!("node_dameon start!");

    if load_identity_config().is_err() {
        error!("load identity config failed!");
        return;
    }

    zone_config = looking_zone_config();
    
    //检查
    etcd_state = check_etcd_by_zone_config(zone_config).await;

    //判断etcd的版本是否大于zone_config里的etcd版本，如果大于说明etcd的数据是新的，否则就要等待 etcd完成更新
    if etcd_state.run_in_this_machine {
        if check_etcd_data() == need_restore{
            restore_etcd_data();//这里是为Zone恢复数据，是独立流程。Node恢复自己的数据
        }

        start_etcd()
    }

    //从系统的高一致性的要重复啊
    node_daemon_main_loop()
    
}
