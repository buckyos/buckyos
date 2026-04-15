# test_control_panel DV 环境测试用例

- 每个文件对应control_panel的一组后台模块，比如test_app_mgr.ts，对应src/frame/control_panel/src/app_service_mgr.rs
- 测试在实现时应直接import src/frame/desktop/src/api/ 下的对应ts封装，而不是自己独立实现
- 测试使用buckyos的AppClient Runtime初始化，每个文件可以通过deno直接启动（runtime状态独立)