import os
import sys
import tempfile
import shutil

# scp to remote server
remote_server = "root@172.20.162.227"
build_dir = os.path.dirname(os.path.abspath(__file__))

source_dir = os.path.join(build_dir, "rootfs/bin")
scp_command = f"scp -r {source_dir} {remote_server}:/opt/buckyos/"
print(scp_command)
os.system(scp_command)

source_dir = os.path.join(build_dir, "rootfs/etc")
scp_command = f"scp -r {source_dir} {remote_server}:/opt/buckyos/"
print(scp_command)
os.system(scp_command)

source_dir = os.path.join(build_dir, "web3_bridge")
scp_command = f"scp -r {source_dir} {remote_server}:/opt/"
print(scp_command)
os.system(scp_command)
