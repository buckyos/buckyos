import os
import shutil

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")

def install():
    # if /opt/buckyos not exist, copy rootfs to /opt/buckyos
    if not os.path.exists("/opt/buckyos"):
        print('copying rootfs to /opt/buckyos')
        shutil.copytree(os.path.join(src_dir, "rootfs"), "/opt/buckyos")
    else:
        print('updating files in /opt/buckyos/bin')
        shutil.rmtree("/opt/buckyos/bin")
        #just update bin
        shutil.copytree(os.path.join(src_dir, "rootfs/bin"), "/opt/buckyos/bin")

if __name__ == "__main__":
    install()