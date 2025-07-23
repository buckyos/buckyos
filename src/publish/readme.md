
# buckyos-nightly channel  的发布流程

## download_pkgs.py $version

- 执行Github Action BuildAll,等待完成
- 下载BuildAll得到的Artifacts(不同平台的rootfs)
- 基于version下载默认app的pkgs
- 解压上述内容到 /opt/buckyos_pkgs/$version/
- 构建必要的 pkg_meta.json到每一个pkg目录

## pack_pkgs.py $version

这一步需要有buckyos.ai的开发者私钥 

- 挨个读取/opt/buckyos_pkgs/$version/下的目录，并调用buckycli的pack_pkg命令
- pack_pkg的结果，放到 /opt/buckyos_pack_pkgs/$version/目录下

## upload_pkgs.py $version

- 将/opt/buckyos_pack_pkgs/$version/下的pkg upload到buckyos.ai的官方repo

## make_office_deb.py / make_win_install.py / make_mac_pkg.py $version

基于/opt/buckyos_pack_pkgs/$version/目录，以及/opt/buckyos_pkgs/$version/目录下的rootfs,构造各个系统的正式版完整安装包。

根据系统机制，这些安装包里携带的Pkg的版本比buckyos.ai的官方Repo的版本更高，因此不会被自动升级。按流程，buckyos的完整安装包总是比自动升级的版本先发布。

## publish_to_repo.py $version

等完整安装包经过了一段时间测试后，就可以推送自动更新了

- 从buckyos.ai的官方repo下载 meta-index.db
- 将 /opt/buckyos_pack_pkgs/$version/目录下的pkg meta加入到meta-index.db中
- 上传新版本的meta-index.db,实现发布（需要buckyos.ai的私钥）
- 发布后，所有订阅nightyly-channel的buckyos系统会收到更新，执行自动升级
