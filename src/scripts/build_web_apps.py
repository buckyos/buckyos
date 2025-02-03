import os
import subprocess

src_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")

def build(dir):
    print(f'building at {dir}')
    work_dir = os.path.join(src_dir, dir)
    subprocess.run(f'pnpm install && pnpm run build', shell=True, cwd=work_dir, check=True)

def build_web_apps():
    print(f'will build web apps')
    build("kernel/node_active")
    build("apps/control_panel/src")
    build("apps/sys_test")

if __name__ == "__main__":
    build_web_apps()
    print(f'build web apps success')