import os
import sys
import tempfile
import shutil

# scp to remote server
remote_server = "root@zhicong.me"
build_dir = os.path.dirname(os.path.abspath(__file__))

source_dir = os.path.join(build_dir, "rootfs/bin")
scp_command = f"scp -r {source_dir} {remote_server}:/opt/buckyos/bin/"
os.system(scp_command)

source_dir = os.path.join(build_dir, "rootfs/etc")
scp_command = f"scp -r {source_dir} {remote_server}:/opt/buckyos/etc/"
os.system(scp_command)

source_dir = os.path.join(build_dir, "web3_bridge")
scp_command = f"scp -r {source_dir} {remote_server}:/opt/"
os.system(scp_command)
