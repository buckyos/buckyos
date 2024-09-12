
import os
import shutil
build_dir = os.path.dirname(os.path.abspath(__file__))
os.system(f"python {build_dir}/killall.py")

print('install files to /opt/buckyos')
shutil.rmtree("/opt/buckyos")
# if /opt/buckyos not exist, copy rootfs to /opt/buckyos
if not os.path.exists("/opt/buckyos"):
    print('copying rootfs to /opt/buckyos')
    shutil.copytree(os.path.join(build_dir, "rootfs"), "/opt/buckyos")
else:
    print('updating files in /opt/buckyos')
    shutil.rmtree("/opt/buckyos/bin")
    #just update bin
    shutil.copytree(os.path.join(build_dir, "rootfs/bin"), "/opt/buckyos/bin")
