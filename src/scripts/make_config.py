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


SCRIPT_DIR = Path(__file__).resolve().parent
ROOTFS_DIR = SCRIPT_DIR.parent / "rootfs"
BUCKYCLI_BIN = ROOTFS_DIR / "bin" / "buckycli" / "buckycli"

try:
    from cert_mgr import CertManager  # type: ignore
except Exception as e:  # pragma: no cover - 仅在缺依赖时打印提示
    CertManager = None
    print(f"warning: cert_mgr import failed: {e}")


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


def make_sn_configs(target_dir: Path) -> None:
    """保留占位，当前不处理 SN 配置。"""
    print("skip sn configs (not implemented)")


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
        }
    raise ValueError(f"invalid group name: {group_name}")

def make_config_by_group_name(group_name: str, target_root: Path, ca_dir: Optional[Path]) -> None:
    params = get_params_from_group_name(group_name)
    print(f"############ make config for group name: {group_name} #########################")
    print(f"rootfs dir : {target_root}")
    print(f"group      : {group_name}")
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
    # SN 构造暂不启用
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

