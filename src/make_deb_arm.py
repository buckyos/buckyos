import os
import sys
import tempfile
import shutil


build_dir = os.path.dirname(os.path.abspath(__file__))

temp_dir = tempfile.gettempdir()

print("make sure YOU already run build.py!!!")

# Copy publish directory
src_dir = os.path.join(build_dir, "publish") 
dest_dir = os.path.join(temp_dir, "deb_build")
if os.path.exists(dest_dir):
    shutil.rmtree(dest_dir)
shutil.copytree(src_dir, dest_dir)
print(f"copy publish to {dest_dir}")

# Copy rootfs directory
src_dir = os.path.join(build_dir, "rootfs")
dest_dir = os.path.join(temp_dir, "deb_build/deb_aarch64/opt/buckyos")
shutil.copytree(src_dir, dest_dir, dirs_exist_ok=True)
print(f"copy rootfs to {dest_dir}")
dest_dir = os.path.join(temp_dir, "deb_build/deb_aarch64/")
print(f"chmod -R 755 {dest_dir}")
os.system("chmod -R 755 " + dest_dir)
clean_dir = os.path.join(temp_dir, "deb_build/deb_aarch64/opt/buckyos/etc")
# Delete .pem and .toml files
delete_cmd = f"cd {clean_dir} && rm -f *.pem *.toml"
os.system(delete_cmd)
print(delete_cmd)


#run dpkg-deb --build mysoftware
dest_dir = os.path.join(temp_dir, "deb_build")
build_cmd = f"cd {dest_dir} && dpkg-deb --build deb_aarch64"
os.system(build_cmd)

print(f"build deb success at {dest_dir}")
copy_cmd = f"cp {dest_dir}/deb_aarch64.deb {build_dir}/buckyos_aarch64.deb"
os.system(copy_cmd)
print(f"copy deb to {build_dir}")
