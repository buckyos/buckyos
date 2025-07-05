//实现一个测试用的main逻辑，不要占用资源，每5秒输出一行日志即可
use buckyos_kit::*;
use log::*;

fn main() {
    init_logging("test_service", true);

    loop {
        info!("Log message");
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}
