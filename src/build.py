
import os
import tempfile
import shutil


build_dir = os.path.dirname(os.path.abspath(__file__))


temp_dir = tempfile.gettempdir()
project_name = "buckyos"
target_dir = os.path.join(temp_dir, "rust_build", project_name)
os.makedirs(target_dir, exist_ok=True)

#cargo_command = f'cargo clean --target-dir "{target_dir}"'
#os.system(cargo_command)

cargo_command = f'cargo build --release --target-dir "{target_dir}"'
build_result = os.system(cargo_command)
if build_result != 0:
    print(f'build failed: {build_result}')
    exit(1)


print(f'build success at: {target_dir}')

print('copying files to rootfs')
destination_dir = os.path.join(build_dir, "rootfs/bin")
shutil.copy(os.path.join(target_dir, "release", "node_daemon"), destination_dir)
destination_dir = os.path.join(build_dir, "rootfs/bin/system_config_service")
shutil.copy(os.path.join(target_dir, "release", "system_config_service"), destination_dir)
destination_dir = os.path.join(build_dir, "rootfs/bin/verify_hub")
shutil.copy(os.path.join(target_dir, "release", "verify_hub"), destination_dir)

print('copying files to rootfs done')

print('install files to /opt/buckyos')
# if /opt/buckyos not exist, copy rootfs to /opt/buckyos
if not os.path.exists("/opt/buckyos"):
    print('copying rootfs to /opt/buckyos')
    shutil.copytree(os.path.join(build_dir, "rootfs"), "/opt/buckyos")
else:
    print('updating files in /opt/buckyos')
    shutil.rmtree("/opt/buckyos/bin")
    #just update bin
    shutil.copytree(os.path.join(build_dir, "rootfs/bin"), "/opt/buckyos/bin")




