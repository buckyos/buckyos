## publish_to_repo.py $version
#- 从buckyos.ai的官方repo下载 meta-index.db
#- 将 /opt/buckyos_pack_pkgs/$version/目录下的pkg meta加入到meta-index.db中
#- 上传新版本的meta-index.db,实现发布（需要buckyos.ai的私钥）

import sys
import os
import glob
import subprocess
import platform
import json
import shutil

target_base_dir = "/opt/buckyos_pack_pkgs"
buckycli_path = os.getenv("BUCKYCLI_PATH", "/opt/buckyos/bin/buckycli/buckycli")

def publish_to_repo(version:str):
    """发布索引使新的index-db生效"""
    cmd = [buckycli_path, "pub_index"]
    print(f"执行命令: {' '.join(cmd)}")
    result = subprocess.run(cmd, capture_output=True, text=True)
    
    if result.returncode == 0:
        print("成功发布索引")
        return True
    else:
        print(f"发布索引失败: {result.stderr}")
        return False

if __name__ == "__main__":
    version = sys.argv[1]
    packed_dirs = glob.glob(os.path.join(target_base_dir, version, "*"))
    publish_to_repo(packed_dirs)