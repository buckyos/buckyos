# cyfs://$this_zone_id/

单机版/桌面版下，`cyfs://$this_zone_id/` 会直接映射到本地的 `data/` 目录。

当前设计下，这里主要承载：

- `home/$userid/`：用户个人数据
- `home/$userid/.local/share/$appid/`：应用永久数据
- `home/$userid/shared/`：用户共享数据
- `srv/library/`：Zone 级共享资料
- `srv/publish/`：Zone 级发布数据
- `srv/$service_name/`：服务持久数据
- `var/$service_name/`：服务运行数据
- `cache/$service_name/`：服务缓存数据

其中用户数据与服务持久数据通常不应在普通升级或覆盖安装时删除。
