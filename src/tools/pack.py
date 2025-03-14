# py实现的打包工具 （用于检测系统读取pkg的兼容性)
# 功能 ：将指定目录打包， pack $src_dir $dest_dir [options]
# 读取src_dir目录下的pkg.meta.json文件，获得必要的元数据信息
# 首先产生.tar.gz文件，放入dest_dir目录下
# 计算tar.gz文件的sha256值，构造完整的pkg.meta.json文件，放入dest_dir目录下
# 如果参数里包含 --sign 选项，则对pkg.meta.json文件进行签名，并保存为pkg.meta.jwt文件
# 尽量减少对外部库的依赖，使用python的标准库实现