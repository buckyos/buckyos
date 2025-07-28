## upload_pkgs.py $version
# - 将/opt/buckyos_pack_pkgs/$version/下的pkg upload到buckyos.ai的官方repo

import sys
import os
import glob
import subprocess


target_base_dir = "/opt/buckyos_pack_pkgs"
buckycli_path = os.getenv("BUCKYCLI_PATH", "/opt/buckyos/bin/buckycli/buckycli")

def upload_pkgs(packed_dirs):
    print(f"publish pack packages in {packed_dirs}")
    cmd = [buckycli_path, "pub_pkg", packed_dirs]
    print(f"执行命令: {' '.join(cmd)}")
    result = subprocess.run(cmd, capture_output=True, text=True)
    
    if result.returncode == 0:
        print("upload packages done")
        return True
    else:
        print(f"upload packages failed: {result.stderr}")
        return False
    
if __name__ == "__main__":
    version = sys.argv[1]
    packed_dirs = glob.glob(os.path.join(target_base_dir, version, "*"))
    upload_pkgs(packed_dirs)