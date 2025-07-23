## pack_pkgs.py $version
#
# 这一步需要有buckyos.ai的开发者私钥 
# - 挨个读取/opt/buckyos_pkgs/$version/下的目录，并调用buckycli的pack_pkg命令
# - pack_pkg的结果，放到 /opt/buckyos_pack_pkgs/$version/目录下