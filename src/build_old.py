
import os
import sys
import tempfile
import shutil


build_dir = os.path.dirname(os.path.abspath(__file__))


temp_dir = tempfile.gettempdir()
project_name = "buckyos"
target_dir = os.path.join(temp_dir, "rust_build", project_name)
os.makedirs(target_dir, exist_ok=True)

args = sys.argv[1:]
if len(args) > 0:
    if args[0] == "clean":
        cargo_command = f'cargo clean --target-dir "{target_dir}"'
        os.system(cargo_command)

os.environ["OPENSSL_STATIC"] = "1"
os.environ["RUSTFLAGS"] = "-C target-feature=+crt-static"
cargo_command = f'cargo build --target x86_64-unknown-linux-musl --release --target-dir "{target_dir}"'
build_result = os.system(cargo_command)
if build_result != 0:
    print(f'build failed: {build_result}')
    exit(1)

target_dir = os.path.join(temp_dir, "rust_build", project_name,"x86_64-unknown-linux-musl")
print(f'build success at: {target_dir}')
print("for buckyos developer: YOU MUST npm build and install buckyos_sdk manually")

npm_build_dir_active = os.path.join(build_dir, "kernel/node_active")
npm_build_cmd = f'cd {npm_build_dir_active} && pnpm run build'
os.system(npm_build_cmd)
print(f'pnpm build success at: {npm_build_dir_active}')

npm_build_dir_control_panel = os.path.join(build_dir, "apps/control_panel/src")
npm_build_cmd = f'cd {npm_build_dir_control_panel} && pnpm run build'
os.system(npm_build_cmd)
print(f'pnpm build success at: {npm_build_dir_control_panel}')

npm_build_dir_sys_test = os.path.join(build_dir, "apps/sys_test")
npm_build_cmd = f'cd {npm_build_dir_sys_test} && pnpm run build'
os.system(npm_build_cmd)
print(f'pnpm build success at: {npm_build_dir_sys_test}')


print('copying files to rootfs')
destination_dir = os.path.join(build_dir, "rootfs/bin")
shutil.copy(os.path.join(target_dir, "release", "node_daemon"), destination_dir)
strip_cmd = f'strip {os.path.join(target_dir, "release", "node_daemon")}'
os.system(strip_cmd)

destination_dir = os.path.join(build_dir, "rootfs/bin/system_config")
shutil.copy(os.path.join(target_dir, "release", "system_config"), destination_dir)
strip_cmd = f'strip {os.path.join(target_dir, "release", "system_config")}'
os.system(strip_cmd)

destination_dir = os.path.join(build_dir, "rootfs/bin/verify_hub")
shutil.copy(os.path.join(target_dir, "release", "verify_hub"), destination_dir)
strip_cmd = f'strip {os.path.join(target_dir, "release", "verify_hub")}'
os.system(strip_cmd)

destination_dir = os.path.join(build_dir, "rootfs/bin/scheduler")
shutil.copy(os.path.join(target_dir, "release", "scheduler"), destination_dir)
strip_cmd = f'strip {os.path.join(target_dir, "release", "scheduler")}'
os.system(strip_cmd)

destination_dir = os.path.join(build_dir, "rootfs/bin/cyfs_gateway")
shutil.copy(os.path.join(target_dir, "release", "cyfs_gateway"), destination_dir)
strip_cmd = f'strip {os.path.join(target_dir, "release", "cyfs_gateway")}'
os.system(strip_cmd)

destination_dir = os.path.join(build_dir, "./web3_bridge/web3_gateway")
shutil.copy(os.path.join(target_dir, "release", "cyfs_gateway"), destination_dir)
strip_cmd = f'strip {os.path.join(target_dir, "release", "cyfs_gateway")}'
os.system(strip_cmd)

destination_dir = os.path.join(build_dir, "rootfs/bin")
shutil.copy(os.path.join(build_dir, "killall.py"), destination_dir)

src_dir = os.path.join(npm_build_dir_active, "dist")
destination_dir = os.path.join(build_dir, "rootfs/bin/active")
os.makedirs(destination_dir, exist_ok=True)
print(f'copying vite build {src_dir} to {destination_dir}')
shutil.rmtree(destination_dir)
shutil.copytree(src_dir, destination_dir)

src_dir = os.path.join(npm_build_dir_control_panel, "dist")
destination_dir = os.path.join(build_dir, "rootfs/bin/control_panel")
os.makedirs(destination_dir, exist_ok=True)
print(f'copying vite build {src_dir} to {destination_dir}')
shutil.rmtree(destination_dir)
shutil.copytree(src_dir, destination_dir)

src_dir = os.path.join(npm_build_dir_sys_test, "dist")
destination_dir = os.path.join(build_dir, "rootfs/bin/sys_test")
os.makedirs(destination_dir, exist_ok=True)
print(f'copying vite build {src_dir} to {destination_dir}')
if os.path.exists(destination_dir):
    shutil.rmtree(destination_dir)
shutil.copytree(src_dir, destination_dir, dirs_exist_ok=True)
print('copying files to rootfs & web3_bridge done')


# if /opt/buckyos not exist, copy rootfs to /opt/buckyos
if not os.path.exists("/opt/buckyos"):
    print('copying rootfs to /opt/buckyos')
    shutil.copytree(os.path.join(build_dir, "rootfs"), "/opt/buckyos")
else:
    print('updating files in /opt/buckyos/bin')
    shutil.rmtree("/opt/buckyos/bin")
    #just update bin
    shutil.copytree(os.path.join(build_dir, "rootfs/bin"), "/opt/buckyos/bin")




