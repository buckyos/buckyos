# 把所有平台的完整rootfs下载到发布机（有buckyos.ai的开发者私钥）

# - 基于Github Action 构建得到rootfs
# - 下载所有平台的rootfs，
# - 下载默认app的pkg,
# - 基于该完整rootfs可以构建不带自动签名的，指定平台的开发版deb(安装包)
