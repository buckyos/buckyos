# 把所有平台的完整rootfs下载到发布机（有buckyos.ai的开发者私钥）

# - 基于Github Action 构建得到rootfs
# - 下载所有平台的rootfs，
# - 下载默认app的pkg,
# - 基于该完整rootfs可以构建不带自动签名的，指定平台的开发版deb(安装包)

import sys
import zipfile
import os
import tarfile

import json
import urllib.request
import urllib.parse
from pathlib import Path

import requests

download_base_dir = Path("/") / "opt" / "buckyosci" / "download"
buckyosci_rootfs_dir = Path("/") / "opt" / "buckyosci" / "rootfs"
system_list = ["windows", "linux", "apple"]
machine_list = ["amd64", "aarch64"]

gh_token = os.environ.get('GITHUB_API_TOKEN')
if not gh_token:
    print("GITHUB_API_TOKEN environment variable is not set.")
    os._exit(1)

def unzip_rootfs(rootfs_path, target_dir):
    """
    解压rootfs压缩包
    zip中如果只有一个rootfs.tar 文件，则需要再解压到target_dir,否则直接解压到target_dir
    """
    if not os.path.exists(rootfs_path):
        print(f"Rootfs file does not exist: {rootfs_path}")
        return
    # 确保目标目录存在
    os.makedirs(target_dir, exist_ok=True)
    
    with zipfile.ZipFile(rootfs_path, 'r') as zip_ref:
        # 获取zip文件中的所有文件列表
        file_list = zip_ref.namelist()
        
        # 检查是否只有一个rootfs.tar文件
        if len(file_list) == 1 and file_list[0].endswith('.tar'):
            # 只有一个tar文件，先解压zip，再解压tar
            zip_ref.extractall(target_dir)
            tar_file_path = os.path.join(target_dir, file_list[0])
            
            # 解压tar文件
            with tarfile.open(tar_file_path, 'r') as tar_ref:
                tar_ref.extractall(target_dir)
            print(f"unzip and tar extract done: {rootfs_path} => {target_dir}")
            # 删除临时tar文件
            os.remove(tar_file_path)
        else:
            # 直接解压zip内容到目标目录
            zip_ref.extractall(target_dir)
            print(f"unzip done: {rootfs_path} => {target_dir}")


def download_from_github_url(github_url):
    art_url = urllib.parse.urlparse(github_url)
    path_parts = art_url.path.strip('/').split('/')
    artifact_id = path_parts[-1]
    owner = path_parts[0]
    repo = path_parts[1]
    gh_api_url = f"https://api.github.com/repos/{owner}/{repo}/actions/artifacts/{artifact_id}/zip"
    
    headers = {
        'Authorization': f'token {gh_token}',
        'Accept': 'application/vnd.github+json',
        'X-GitHub-Api-Version': '2022-11-28'
    }
    resp = requests.get(gh_api_url, headers=headers, stream=True, allow_redirects=True)

    if resp.ok:
        file_length = resp.headers.get('content-length')
        print("\tFile length:", file_length)
        
        content_disposition = resp.headers.get('content-disposition')
        if content_disposition and 'filename=' in content_disposition:
            file_name = content_disposition.split('filename=')[1].replace('"', '')
        else:
            file_name = 'artifact.zip'
        print("\tFile name:", file_name)
        save_path = download_base_dir / file_name
        print("\tSaving to file:", save_path)
        with open(save_path, 'wb') as f:
            for chunk in resp.iter_content(chunk_size=8192):
                f.write(chunk)
    else:
        print(f"Failed to download artifact from {gh_api_url}, status code: {resp.status_code}")
        return
        
        

def download_rootfs(version):
    # do download
    # get github artifact url from official test server
    query_url = f"https://buckyos.ai/version/?version={urllib.parse.quote(version)}"
    print(f"Qureying artifacts from official test server...")
    with urllib.request.urlopen(query_url) as response:
        if response.status != 200:
            print(f"Failed to query version {version}, status code: {response.status}")
            return
        data = response.read().decode('utf-8')
        versions = json.loads(data)
        for item in versions['items']:
            print(f"downloading buckyos: {item['os']}-{item['arch']}-{item['version']}")
            download_from_github_url(item['url'])
    # unzip rootfs
    
    for os_name in system_list:
        for machine_name in machine_list:
            if os_name == "windows" and machine_name == "aarch64":
                continue
            rootfs_id = f"buckyos-{os_name}-{machine_name}"
            target_dir = os.path.join(buckyosci_rootfs_dir, version, rootfs_id)
            if not os.path.exists(target_dir):
                os.makedirs(target_dir)
            zipfile = os.path.join(download_base_dir, f"buckyos-{os_name}-{machine_name}-{version}.zip")
            unzip_rootfs(zipfile, target_dir)


    pass

if __name__ == "__main__":
    #version = "0.4.0-250724"
    version = sys.argv[1]
    if not os.path.exists(download_base_dir):
        os.makedirs(download_base_dir)
    if not os.path.exists(buckyosci_rootfs_dir):
        os.makedirs(buckyosci_rootfs_dir)
    download_rootfs(version)
    print("download_rootfs done")


