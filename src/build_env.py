import os
import sys
import tempfile
import shutil

build_dir = os.path.dirname(os.path.abspath(__file__))

npm_buckyos_dir = os.path.join(build_dir, "kernel/buckyos_sdk")
npm_build_cmd = f'cd {npm_buckyos_dir} && pnpm install && pnpm run build'
os.system(npm_build_cmd)

npm_build_dir_active = os.path.join(build_dir, "kernel/node_active")
npm_build_cmd = f'cd {npm_build_dir_active} && pnpm install && pnpm run build'
os.system(npm_build_cmd)

npm_build_dir_control_panel = os.path.join(build_dir, "apps/control_panel/src")
npm_build_cmd = f'cd {npm_build_dir_control_panel} && pnpm install && pnpm run build'
os.system(npm_build_cmd)


npm_build_dir_sys_test = os.path.join(build_dir, "apps/sys_test")
npm_build_cmd = f'cd {npm_build_dir_sys_test} && pnpm install && pnpm run build'
os.system(npm_build_cmd)

print(f'pnpm install success at: {build_dir}')
