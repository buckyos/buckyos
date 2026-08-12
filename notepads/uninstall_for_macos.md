# BuckyOS macOS 手工卸载说明

本文说明 macOS pkg 安装后的手工卸载步骤。BuckyOS macOS pkg 本身不提供卸载入口，也不依赖 pkg 内的 `uninstall` script 自动执行卸载。

## 默认策略

- 默认只删除程序文件、服务文件、命令行工具和可再生运行缓存。
- 默认保留用户配置和数据。
- 只有在用户明确执行“删除用户数据”步骤时，才删除 `apps.buckyos.data_paths` 对应内容。

## 停止服务和相关进程

```bash
sudo launchctl bootout system /Library/LaunchDaemons/buckyos.service.plist 2>/dev/null || true
sudo pkill -f '/opt/buckyos/bin/node-daemon/node_daemon' 2>/dev/null || true
containers=$(docker ps -aq --filter "label=buckyos.full_appid" 2>/dev/null || true)
if [ -n "$containers" ]; then
  docker stop $containers
fi
```

## 删除程序文件

```bash
sudo rm -f /Library/LaunchDaemons/buckyos.service.plist
sudo rm -rf /opt/buckyos/bin
sudo rm -rf /opt/buckyos/local
sudo rm -rf /opt/buckyos/logs
sudo rm -rf /opt/buckyos/data/var
sudo rm -rf /opt/buckyos/data/cache
```

删除桌面应用和 CLI：

```bash
sudo rm -rf /Applications/BuckyOS.app
rm -rf "$HOME/.buckycli"
```

如果安装流程向 shell profile 写入了 `buckycli` PATH，需要从对应 profile 文件中手工移除。

## 可选：删除用户数据

以下路径默认保留。只有在确认不再需要本机 BuckyOS 数据时才执行：

```bash
sudo rm -rf /opt/buckyos/etc
sudo rm -rf /opt/buckyos/data
sudo rm -rf /opt/buckyos/storage
rm -rf "$HOME/Library/Application Support/BuckyOSApp"
```

## 可选：清理 pkg receipt

如果需要让系统不再记录 BuckyOS pkg receipt，可以先查询 package id，再执行 `pkgutil --forget`：

```bash
pkgutil --pkgs | grep -i buckyos
sudo pkgutil --forget <package-id>
```
