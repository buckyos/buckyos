## publish_to_repo.py $version
#- 从buckyos.ai的官方repo下载 meta-index.db
#- 将 /opt/buckyos_pack_pkgs/$version/目录下的pkg meta加入到meta-index.db中
#- 上传新版本的meta-index.db,实现发布（需要buckyos.ai的私钥）