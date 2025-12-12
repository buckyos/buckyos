# rootfs的构建
# 1. git clone 后,rootfs里只有 “必要的代码文件”，（相关配置文件也是以代码的形式存在的）
# 2. build 后，rootfs里的bin目录会填充正确的编译产物
# ------- start.py里的内容
# 3. 基于该rootfs(主要是buckycli工具)调用make_config.py $config_group_name 会在rootfs里完成所有的配置文件
# 4. 基于完成构建的rootfs,可以制作安装包，或则复制到开发环境运行调试（本机调试或虚拟机调试) --> 总是可以通过观察rootfs里的配置文件来了解上一次运行的配置
# 5. 对于有多个node的虚拟机环境，是在完成了Linux版本的构建后，基于不同环境的需要make_config.py $node_group_name 来构造不同的rootfs,并复制到对应的虚拟机里
#
# 需要构造的配置文件列表
# - rootfs/local/did_docs/ 放入必要的doc缓存
# - rootfs/node_daemon/root_pkg_env/pkgs/meta_index.db.fileobj 本机自动更新的“最后更新时间缓存”，该文件确保不会触发自动更新
# - rootfs/etc/machine.json 根据目标环境的web3网桥配置和可信发行者，进行配置
# - rootfs/etc/激活的身份文件组，(start_config.json,$zoneid.zone.json,node_identity.json,node_private_key.pem,tls证书文件，.buckycli目录下的ownerconfig)
#
# SN的文件结构与标准的ood不同
# - 有必要的身份文件组
# - 必须支持DNS解析，需要特定的配置文件（为了防止混淆,sn使用web3_gateway作为配置文件的入口
# - 需要根据需要构造sn_db（模拟用户注册）
# - 提供source repo服务（另一个子域名）, 提供订阅用户的系统自动更新
#
# 直接使用 buckycli 与 cert_mgr 在 rootfs 内构造所有配置，不从现有目录复制。
# SN 相关仍保留占位，sn_db 不做构造。

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path
import time
from typing import Dict, Iterable, List, Optional, Tuple
from util import get_buckyos_root
from cert_mgr import CertManager  # type: ignore

SCRIPT_DIR = Path(__file__).resolve().parent
ROOTFS_DIR = SCRIPT_DIR.parent / "rootfs"
BUCKYCLI_BIN = ROOTFS_DIR / "bin" / "buckycli" / "buckycli"
if not BUCKYCLI_BIN.exists():
    BUCKYCLI_BIN = Path(get_buckyos_root()) / "bin" / "buckycli" / "buckycli"
    if not BUCKYCLI_BIN.exists():
        raise FileNotFoundError(f"buckycli binary missing at {BUCKYCLI_BIN}")

print(f"* buckycli at {BUCKYCLI_BIN}")


def ensure_dir(path: Path) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    return path


def run_cmd(cmd: List[str], cwd: Optional[Path] = None) -> None:
    result = subprocess.run(
        cmd,
        cwd=str(cwd) if cwd is not None else None,
        text=True,
        capture_output=True,
    )
    if result.stdout:
        print(result.stdout)
    if result.stderr:
        print(result.stderr, file=sys.stderr)
    if result.returncode != 0:
        raise RuntimeError(f"command failed: {' '.join(cmd)}")


def run_buckycli(args: List[str]) -> None:
    cmd = [str(BUCKYCLI_BIN)] + args
    run_cmd(cmd, cwd=ROOTFS_DIR)


def copy_if_exists(src: Path, dst: Path) -> None:
    if not src.exists():
        print(f"skip missing file: {src}")
        return
    ensure_dir(dst.parent)
    shutil.copy2(src, dst)
    print(f"copy {src} -> {dst}")


def write_json(path: Path, data: dict) -> None:
    ensure_dir(path.parent)
    path.write_text(json.dumps(data, indent=2))
    print(f"write json {path}")


def make_global_env_config(
    target_dir: Path,
    web3_bns: str,
    trust_did: Iterable[str],
    force_https: bool,
) -> None:
    """写入机器级配置和默认 meta_index 缓存。"""
    etc_dir = ensure_dir(target_dir / "etc")

    machine = {
        "web3_bridge": {"bns": web3_bns},
        "force_https": force_https,
        "trust_did": list(trust_did),
    }
    write_json(etc_dir / "machine.json", machine)

    meta_dst = (
        target_dir
        / "local"
        / "node_daemon"
        / "root_pkg_env"
        / "pkgs"
        / "meta_index.db.fileobj"
    )

    ensure_dir(meta_dst.parent)
    now_unix_time = int(time.time())
    meta_dst.write_text(
        json.dumps({"name":"test.data","size":100,"content":"sha256:1234567890","create_time":now_unix_time}, indent=2)
    )
    print(f"create default meta_index cache at {meta_dst}")


def make_cache_did_docs(target_dir: Path) -> None:
    """通过 buckycli 构造 did_docs（依赖未来的 build_did_docs 实现）。"""
    docs_dst = target_dir / "local" / "did_docs"

    ensure_dir(docs_dst)
    try:
        run_buckycli(["build_did_docs", "--output_dir", str(docs_dst)])
    except RuntimeError as e:
        print(f"warning: build_did_docs not available yet: {e}")


def _copy_identity_outputs(
    user_dir: Path, node_dir: Path, target_dir: Path, zone_id: str
) -> None:
    etc_dir = ensure_dir(target_dir / "etc")

    copy_if_exists(user_dir / f"{zone_id}.zone.json", etc_dir / f"{zone_id}.zone.json")
    for name in ("start_config.json", "node_identity.json", "node_private_key.pem"):
        copy_if_exists(node_dir / name, etc_dir / name)

    buckycli_dir = ensure_dir(etc_dir / ".buckycli")
    for name in ("user_config.json", "user_private_key.pem"):
        copy_if_exists(user_dir / name, buckycli_dir / name)
    copy_if_exists(user_dir / f"{zone_id}.zone.json", buckycli_dir / "zone_config.json")


def _generate_tls(zone_id: str, ca_name: str, etc_dir: Path, ca_dir: Optional[Path]) -> None:
    if CertManager is None:
        print("warning: cert_mgr not available, skip TLS cert generation")
        return

    cm = CertManager()
    # 优先使用用户提供的 CA 目录
    if ca_dir:
        ca_dir_path = ca_dir.resolve()
        if not ca_dir_path.exists():
            raise FileNotFoundError(f"CA dir not found: {ca_dir_path}")
        ca_cert_candidates = list(ca_dir_path.glob("*_ca_cert.pem"))
        if not ca_cert_candidates:
            raise FileNotFoundError(f"no *_ca_cert.pem in {ca_dir_path}")
        ca_cert_path = ca_cert_candidates[0]
        ca_key_path = ca_dir_path / ca_cert_path.name.replace("_ca_cert.pem", "_ca_key.pem")
        if not ca_key_path.exists():
            raise FileNotFoundError(f"CA key not found: {ca_key_path}")
    else:
        cert_dir = ensure_dir(etc_dir / "certs")
        ca_cert, ca_key = cm.create_ca(str(cert_dir), name=ca_name)
        ca_cert_path, ca_key_path = Path(ca_cert), Path(ca_key)

    cert_path, key_path = cm.create_cert_from_ca(
        str(ca_dir_path if ca_dir else ca_cert_path.parent),
        hostname=zone_id,
        hostnames=[zone_id, f"*.{zone_id}"],
        target_dir=str(etc_dir),
    )

    # 兼容老命名，仅保留一套证书（包含 zone 与通配 SAN）
    copy_if_exists(Path(cert_path), etc_dir / "tls_certificate.pem")
    copy_if_exists(Path(key_path), etc_dir / "tls_key.pem")
    # 保留 CA 以便信任
    copy_if_exists(ca_cert_path, etc_dir / "ca_certificate.pem")
    copy_if_exists(ca_key_path, etc_dir / "ca_key.pem")
    print(f"tls certs generated under {etc_dir}")


def make_identity_files(
    target_dir: Path,
    username: str,
    zone_id: str,
    node_name: str,
    sn_base_host: str,
    ca_name: str,
    ca_dir: Optional[Path],
) -> None:
    """使用 buckycli 生成身份文件，并利用 cert_mgr 生成 TLS 证书。"""
    if not BUCKYCLI_BIN.exists():
        raise FileNotFoundError(f"buckycli binary missing at {BUCKYCLI_BIN}")

    tmp_root = ensure_dir(target_dir / "_buckycli_tmp")
    user_tmp = ensure_dir(tmp_root / zone_id)

    # 1. 创建 user/zone
    run_buckycli(
        [
            "create_user_env",
            "--username",
            username,
            "--hostname",
            zone_id,
            "--ood_name",
            node_name,
            "--sn_base_host",
            sn_base_host,
            "--output_dir",
            str(user_tmp),
        ]
    )

    # 2. 创建节点配置
    run_buckycli(
        [
            "create_node_configs",
            "--device_name",
            node_name,
            "--env_dir",
            str(user_tmp),
        ]
    )

    # 3. 拷贝身份文件
    user_dir = user_tmp
    node_dir = user_dir / node_name
    _copy_identity_outputs(user_dir, node_dir, target_dir, zone_id)

    # 4. TLS 证书
    _generate_tls(zone_id, ca_name, ensure_dir(target_dir / "etc"), ca_dir)


def make_repo_cache_file(target_dir: Path) -> None:
    """写入 meta_index 缓存文件（占位防止自动更新）。"""
    meta_dst = (
        target_dir
        / "local"
        / "node_daemon"
        / "root_pkg_env"
        / "pkgs"
        / "meta_index.db.fileobj"
    )
    if not meta_dst.exists():
        ensure_dir(meta_dst.parent)
        meta_dst.write_text(
            '{"content":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","name":"meta_index.db","size":53248}'
        )
        print(f"create default meta_index cache at {meta_dst}")


def make_sn_configs(
    target_dir: Path,
    sn_base_host: str,
    sn_ip: str,
    sn_device_name: str = "sn_server",
    ca_name: str = "buckyos_sn",
    ca_dir: Optional[Path] = None,
) -> None:
    """生成 SN（Super Node）服务器配置文件。
    
    所有配置文件直接平铺在 target_dir 目录下，包括：
    - sn_server_private_key.pem - rtcp 协议栈用到的设备私钥文件
    - fullchain.cert, fullchain.pem - 包含 sn.$sn_base, *.web3.$sn_base 的证书和密钥
    - ca/buckyos_sn_ca_cert.pem, ca/buckyos_sn_ca_key.pem - 测试环境自签名 CA 证书
    - zone_zone - 自动生成的，包含 buckyos 定制的 DNS TXT 记录模板
    
    注意：以下文件需要用户手动创建，不由本脚本生成：
    - dns_zone - 手工配置的 DNS Zone 文件
    - website.yaml - 被 web3_gateway 引用的网站配置文件
    
    Args:
        target_dir: 输出目录，所有文件直接平铺在此目录下
        sn_base_host: SN 基础域名（例如 buckyos.io 或 devtests.org）
        sn_ip: SN 服务器 IP 地址
        sn_device_name: SN 设备名称，默认 "sn_server"
        ca_name: CA 证书名称
        ca_dir: 使用已有 CA 目录，否则自动生成
    """
    if not BUCKYCLI_BIN.exists():
        raise FileNotFoundError(f"buckycli binary missing at {BUCKYCLI_BIN}")
    
    print(f"生成 SN 配置文件到 {target_dir} ...")
    print(f"  SN 基础域名: {sn_base_host}")
    print(f"  SN IP 地址: {sn_ip}")
    print(f"  SN 设备名称: {sn_device_name}")
    
    # SN 配置文件直接平铺在 target_dir 下，不创建 etc 子目录
    ensure_dir(target_dir)
    
    # 1. 使用 buckycli 创建 SN 配置
    # 注意：SN 使用特殊的身份，这里使用 buckycli 的 create_sn_configs 命令
    print("# 步骤 1: 创建 SN 设备身份配置...")
    run_buckycli(
        [
            "create_sn_configs",
            "--output_dir",
            str(target_dir),
            "--sn_ip",
            sn_ip,
            "--sn_base_host",
            sn_base_host,
        ]
    )
    
    # buckycli 会在 target_dir/sn_server/ 下生成文件，需要移动到 target_dir
    buckycli_sn_dir = target_dir / "sn_server"
    if buckycli_sn_dir.exists():
        # 移动生成的文件到 target_dir 根目录
        for file in buckycli_sn_dir.glob("*"):
            if file.is_file():
                dest_file = target_dir / file.name
                shutil.move(str(file), str(dest_file))
                print(f"移动文件: {file.name} -> {target_dir}/")
        # 删除空的 sn_server 目录
        if buckycli_sn_dir.exists() and not list(buckycli_sn_dir.iterdir()):
            buckycli_sn_dir.rmdir()

    
    # 2. 生成 TLS 证书
    print("# 步骤 2: 生成 TLS 证书...")

    cm = CertManager()
    
    # 生成或使用已有 CA
    if ca_dir and ca_dir.exists():
        ca_dir_path = ca_dir.resolve()
        print(f"使用已有 CA: {ca_dir_path}")
        ca_cert_candidates = list(ca_dir_path.glob("*_ca_cert.pem"))
        if not ca_cert_candidates:
            raise FileNotFoundError(f"在 {ca_dir_path} 中未找到 *_ca_cert.pem")
        ca_cert_path = ca_cert_candidates[0]
        ca_key_path = ca_dir_path / ca_cert_path.name.replace("_ca_cert.pem", "_ca_key.pem")
        if not ca_key_path.exists():
            raise FileNotFoundError(f"CA 私钥未找到: {ca_key_path}")
    else:
        # 生成新的 CA
        ca_output_dir = ensure_dir(target_dir / "ca")
        ca_cert, ca_key = cm.create_ca(str(ca_output_dir), name=ca_name)
        ca_cert_path, ca_key_path = Path(ca_cert), Path(ca_key)
        print(f"已生成 CA 证书: {ca_cert_path}")
    
    # 生成服务器证书（包含 sn.$sn_base 和 *.web3.$sn_base）
    sn_hostname = f"sn.{sn_base_host}"
    web3_wildcard = f"*.web3.{sn_base_host}"
    
    cert_path, key_path = cm.create_cert_from_ca(
        str(ca_cert_path.parent),
        hostname=sn_hostname,
        target_dir=str(target_dir),
        hostnames=[sn_hostname, web3_wildcard, f"web3.{sn_base_host}"],
    )
    
    # 复制/重命名为标准文件名
    cert_file = Path(cert_path)
    key_file = Path(key_path)
    
    shutil.copy2(cert_file, target_dir / "fullchain.cert")
    shutil.copy2(key_file, target_dir / "fullchain.pem")
    
    # 复制 CA 证书到 ca 目录（用于客户端信任）
    if ca_dir:
        ca_output_dir = ensure_dir(target_dir / "ca")
        shutil.copy2(ca_cert_path, ca_output_dir / ca_cert_path.name)
        shutil.copy2(ca_key_path, ca_output_dir / ca_key_path.name)
    
    print(f"TLS 证书已生成:")
    print(f"  - {target_dir / 'fullchain.cert'}")
    print(f"  - {target_dir / 'fullchain.pem'}")
    print(f"  - {target_dir / 'ca' / ca_cert_path.name}")
    
    #3 修改params.json
    params_json = {
        "params": {
            "sn_host": sn_base_host,
            "sn_ip": sn_ip,
            "sn_boot_jwt": "todo",
            "sn_owner_pk": "todo",
            "sn_device_jwt": "todo",
        }
    }
    
    print(f"\n✓ SN 配置文件生成完成!")
    print(f"  输出目录: {target_dir}")
    print(f"\n生成的文件:")
    print(f"  - {target_dir / 'sn_private_key.pem'} (设备私钥)")
    print(f"  - {target_dir / 'fullchain.cert'} (服务器证书)")
    print(f"  - {target_dir / 'fullchain.pem'} (服务器私钥)")
    print(f"  - {target_dir / 'zone_zone.toml'} (BuckyOS DNS TXT 记录，会动态更新)")
    print(f"  - {target_dir / 'ca' / 'buckyos_sn_ca_cert.pem'} (CA 证书)")
    print(f"  - {target_dir / 'params.json'} (SN 配置参数)")
    print(f"\n需要手动创建的文件:")
    print(f"  - {target_dir / 'dns_zone'} (DNS Zone 配置)")
    print(f"  - {target_dir / 'website.yaml'} (网站配置)")
    print(f"\n其他注意事项:")
    print(f"  - 测试环境需要将 CA 证书安装到客户端信任列表")


def make_sn_db(target_dir: Path, user_list: List[str]) -> None:
    """占位，按需求补充。"""
    print("skip sn_db generation (not implemented)")


def get_params_from_group_name(group_name: str) -> Dict[str, object]:
    """根据分组名获取所有生成参数。"""
    if group_name == "dev":
        return {
            "username": "devtest",
            "zone_id": "test.buckyos.io",
            "node_name": "ood1",
            "netid": "",
            "sn_base_host": "",
            "web3_bridge": "web3.devtests.org",
            "trust_did": [
                "did:web:buckyos.org",
                "did:web:buckyos.ai",
                "did:web:buckyos.io",
            ],
            "force_https": False,
            "ca_name": "buckyos_local",
            "is_sn": False,
        }
    if group_name == "alice.ood1":
        return {
            "username": "alice",
            "zone_id": "alice.web3.devtests.org",
            "node_name": "ood1",
            "netid": "",
            "sn_base_host": "devtests.org",
            "web3_bridge": "web3.devtests.org",
            "trust_did": [
                "did:web:buckyos.org",
                "did:web:buckyos.ai",
                "did:web:buckyos.io",
            ],
            "force_https": False,
            "ca_name": "buckyos_local",
            "is_sn": False,
        }
    if group_name == "bob.ood1":
        return {
            "username": "bob",
            "zone_id": "bob.web3.devtests.org",
            "node_name": "ood1",
            "netid": "",
            "sn_base_host": "devtests.org",
            "web3_bridge": "web3.devtests.org",
            "trust_did": [
                "did:web:buckyos.org",
                "did:web:buckyos.ai",
                "did:web:buckyos.io",
            ],
            "force_https": False,
            "ca_name": "buckyos_local",
            "is_sn": False,
        }
    if group_name == "sn_server":
        return {
            "sn_base_host": "devtests.org",
            "sn_ip": "127.0.0.1",
            "sn_device_name": "sn_server",
            "web3_bridge": "web3.devtests.org",
            "trust_did": [
                "did:web:buckyos.org",
                "did:web:buckyos.ai",
                "did:web:buckyos.io",
            ],
            "force_https": False,
            "ca_name": "buckyos_sn",
            "is_sn": True,
        }
    raise ValueError(f"invalid group name: {group_name}")

def make_config_by_group_name(group_name: str, target_root: Path, ca_dir: Optional[Path]) -> None:
    params = get_params_from_group_name(group_name)
    print(f"############ make config for group name: {group_name} #########################")
    print(f"rootfs dir : {target_root}")
    print(f"group      : {group_name}")
    
    is_sn = params.get("is_sn", False)
    
    if is_sn:
        # SN 配置生成
        print(f"sn_base_host: {params['sn_base_host']}")
        print(f"sn_ip       : {params['sn_ip']}")
        print(f"device_name : {params['sn_device_name']}")
        print(f"web3_bridge : {params['web3_bridge']}")
        
        # SN 不需要 machine.json、did_docs 缓存和 meta_index 缓存
        make_sn_configs(
            target_root,
            params["sn_base_host"],
            params["sn_ip"],
            params["sn_device_name"],
            params["ca_name"],
            ca_dir,
        )
    else:
        # 普通 OOD 节点配置生成
        print(f"username   : {params['username']}")
        print(f"zone       : {params['zone_id']}")
        print(f"node       : {params['node_name']}")
        print(f"web3_bridge: {params['web3_bridge']}")
        
        make_global_env_config(
            target_root,
            params["web3_bridge"],
            params["trust_did"],
            params["force_https"],
        )
        
        make_cache_did_docs(target_root)
        make_identity_files(
            target_root,
            params["username"],
            params["zone_id"],
            params["node_name"],
            params["sn_base_host"],
            params["ca_name"],
            ca_dir,
        )
        make_repo_cache_file(target_root)
    
    print(f"config {group_name} generation finished.")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="在 rootfs 下生成配置文件",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    parser.add_argument("group", help="配置分组名，例如 dev")
    parser.add_argument(
        "--rootfs",
        default=ROOTFS_DIR,
        type=Path,
        help="输出目录（包含 bin/buckycli 等工具）",
    )
    parser.add_argument(
        "--ca",
        default=None,
        type=Path,
        help="使用已有 CA 目录（含 *_ca_cert.pem 与对应 key），否则自动生成",
    )
    args = parser.parse_args()
    make_config_by_group_name(args.group, args.rootfs, args.ca)

if __name__ == "__main__":
    sys.exit(main())

