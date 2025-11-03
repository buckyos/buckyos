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

buckyosci_root = os.environ.get('BUCKYOS_BUILD_ROOT', "/opt/buckyosci")
print(f"Using BUCKYOS_BUILD_ROOT: {buckyosci_root}")

download_base_dir = os.path.join(buckyosci_root, "download")
buckyosci_rootfs_dir = os.path.join(buckyosci_root, "rootfs")
system_list = ["windows", "linux", "apple"]
machine_list = ["amd64", "aarch64"]

gh_token = os.environ.get('GITHUB_API_TOKEN')
if not gh_token:
    # read token from file ~/.github_api_token
    token_file = Path.home() / ".github_api_token"
    if token_file.exists():
        with open(token_file, 'r') as f:
            gh_token = f.read().strip()

if not gh_token:
    print("GITHUB_API_TOKEN is not set, please set it or create ~/.github_api_token file with your token.")
    sys.exit(1)

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
        save_path = os.path.join(download_base_dir, file_name)
        if os.path.exists(save_path):
            print(f"File {save_path} already exists, checking length.")
            existing_size = os.path.getsize(save_path)
            if existing_size == int(file_length):
                print(f"File {save_path} already exists and is complete, skipping download.")
                return save_path
            else:
                print(f"File {save_path} exists but is incomplete, remove and downloading again.")
                os.remove(save_path)
        print("\tSaving to file:", save_path)
        with open(save_path, 'wb') as f:
            for chunk in resp.iter_content(chunk_size=8192):
                f.write(chunk)
        
        return save_path
    else:
        print(f"Failed to download artifact from {gh_api_url}, status code: {resp.status_code}")
        return None
        
        

def download_rootfs(version, os_str, arch):
    # do download
    # get github artifact url from official test server
    query_url = f"https://buckyos.ai/version/?version={urllib.parse.quote(version)}"
    if os_str:
        query_url += f"&os={urllib.parse.quote(os_str)}"
    if arch:
        query_url += f"&arch={urllib.parse.quote(arch)}"
    print(f"Qureying artifacts from official test server...")
    with urllib.request.urlopen(query_url) as response:
        if response.status != 200:
            print(f"Failed to query version {version}, status code: {response.status}")
            return
        data = response.read().decode('utf-8')
        versions = json.loads(data)
        for item in versions['items']:
            if item['os'] == "windows" and item['arch'] == "aarch64":
                print("Skipping windows aarch64 rootfs")
                continue
            rootfs_id = f"buckyos-{item['os']}-{item['arch']}"
            target_dir = os.path.join(buckyosci_rootfs_dir, version, rootfs_id)
            if os.path.exists(target_dir):
                print(f"Rootfs already exists for {rootfs_id}, skipping download.")
                continue
            print(f"downloading buckyos: {item['os']}-{item['arch']}-{item['version']}")
            zipfile = download_from_github_url(item['url'])
            if not zipfile:
                print(f"Failed to download artifact for {item['os']}-{item['arch']}")
                break
            
            print(f"Extracting rootfs for {rootfs_id}")
            if not os.path.exists(target_dir):
                os.makedirs(target_dir)
            unzip_rootfs(zipfile, target_dir)

if __name__ == "__main__":
    #version = "0.4.0+build250724"
    version = sys.argv[1]
    os_str = sys.argv[2] if len(sys.argv) > 2 else None
    arch = sys.argv[3] if len(sys.argv) > 3 else None
    if not os.path.exists(download_base_dir):
        os.makedirs(download_base_dir)
    if not os.path.exists(buckyosci_rootfs_dir):
        os.makedirs(buckyosci_rootfs_dir)
    download_rootfs(version, os_str, arch)
    print("download_rootfs done")


