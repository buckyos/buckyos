import os

def build_app_image():
    os.system("docker build -t buckyos/hello_bucky ./src")
    # 保存到本地
    os.system("docker save -o hello_bucky_img_x86.tar buckyos/hello_bucky")
    # 修改权限为非sudo用户也可以正常访问
    os.system("chmod 555 hello_bucky_img_x86.tar")
    # 保存到rootfs/tmp/app_images
    os.system("mv ./hello_bucky_img_x86.tar ../../rootfs/tmp/app_images")
    # 删除镜像
    os.system("docker rmi buckyos/hello_bucky")

if __name__ == "__main__":
    build_app_image()
    