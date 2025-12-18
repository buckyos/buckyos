# rootfs construction process
# 1. After git clone, rootfs contains only "essential code files" (related config files also exist as code)
# 2. After build, the rootfs/bin directory will be filled with correct build artifacts
# ------- Content from start.py
# 3. Based on this rootfs (mainly buckycli tool), calling make_config.py $config_group_name will complete all config files in target_rootfs
# 4. Based on the completed rootfs, you can make installation packages or copy to development environment for debugging (local or VM debugging) --> You can always understand the last run configuration by observing the config files in rootfs
# 5. For VM environments with multiple nodes, after completing the Linux version build, use make_config.py $node_group_name based on different environment needs to construct different rootfs and copy to corresponding VMs
#
# List of configuration files to be constructed
# - rootfs/local/did_docs/ - put necessary doc cache
# - rootfs/node_daemon/root_pkg_env/pkgs/meta_index.db.fileobj - local auto-update "last update time cache", this file ensures no auto-update is triggered
# - rootfs/etc/machine.json - configure according to target environment's web3 bridge and trusted issuers
# - rootfs/etc/activated identity file group (start_config.json, $zoneid.zone.json, node_identity.json, node_private_key.pem, TLS certificate files, ownerconfig under .buckycli directory)
#
# SN file structure is different from standard OOD
# - Has necessary identity file group
# - Must support DNS resolution, needs specific config files (to prevent confusion, SN uses web3_gateway as config file entry point)
# - Need to construct sn_db as needed (simulate user registration)
# - Provide source repo service (another subdomain), provide system auto-update for subscribed users
#
# Use buckycli and cert_mgr directly to construct all configurations in rootfs, not copy from existing directory.
# SN related still reserved as placeholder, sn_db not constructed.

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path
import time
from typing import Dict, Iterable, List, Optional, Tuple
from buckyos_devkit.buckyos_kit import get_buckyos_root,get_execute_name
from buckyos_devkit import CertManager  # type: ignore


PROJECT_DIR = Path(__file__).resolve().parent
ROOTFS_DIR = Path(get_buckyos_root()) # rootfs is default target dir.
BUCKYCLI_BIN =Path(get_execute_name(Path("~/buckycli/buckycli").expanduser()))

if not BUCKYCLI_BIN.exists():
    print(f"buckycli binary missing at {BUCKYCLI_BIN}")
    print("use 'python3 build.py' to build and install buckycli to your home directory" )
    raise FileNotFoundError(f"buckycli binary missing at {BUCKYCLI_BIN}")

BUCKYCLI_DIR = BUCKYCLI_BIN.parent
print(f"* buckycli = {BUCKYCLI_BIN}")


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

def write_text(path: Path, content: str) -> None:
    ensure_dir(path.parent)
    path.write_text(content)
    print(f"write content {path}")


def make_global_env_config(
    target_dir: Path,
    web3_bns: str,
    trust_did: Iterable[str],
    force_https: bool,
) -> None:
    """Write machine-level configuration and default meta_index cache."""
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
    """Construct did_docs via buckycli (depends on future build_did_docs implementation)."""
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
    for name in ("start_config.json", "node_identity.json", "node_private_key.pem", "node_device_config.json"):
        copy_if_exists(node_dir / name, etc_dir / name)

    buckycli_dir = ensure_dir(etc_dir / ".buckycli")
    for name in ("user_config.json", "user_private_key.pem"):
        copy_if_exists(user_dir / name, buckycli_dir / name)
    copy_if_exists(user_dir / f"{zone_id}.zone.json", buckycli_dir / "zone_config.json")

def _check_or_generate_ca(cm: CertManager, ca_name: str, ca_dir: Path) -> None:
    # Generate or use existing CA
    ca_dir_path = ca_dir.resolve()
    ensure_dir(ca_dir_path)
    print(f"Check CA at : {ca_dir_path}")
    ca_cert_path = ca_dir_path / f"{ca_name}_ca_cert.pem"
    ca_key_path = ca_dir_path / f"{ca_name}_ca_key.pem"

    if ca_cert_path.exists() and ca_key_path.exists():
        print(f"Use existing CA at : {ca_cert_path}")
        return ca_cert_path, ca_key_path
    else:
        print(f"Generate new CA at : {ca_dir_path}")
        ca_cert, ca_key = cm.create_ca(str(ca_dir_path), name=ca_name)
        ca_cert_path, ca_key_path = Path(ca_cert), Path(ca_key)

    return ca_cert_path, ca_key_path

def _generate_tls(zone_id: str, ca_name: str, etc_dir: Path, ca_dir: Path) -> None:
    if CertManager is None:
        print("warning: cert_mgr not available, skip TLS cert generation")
        return

    cm = CertManager()
    ca_cert_path, ca_key_path = _check_or_generate_ca(cm, ca_name, ca_dir)
    cert_path, key_path = cm.create_cert_from_ca(
        str(ca_dir),
        hostname=zone_id,
        hostnames=[zone_id, f"*.{zone_id}"],
        target_dir=str(etc_dir),
    )

    shutil.move(cert_path, etc_dir / "zone_cert.cert")
    shutil.move(key_path, etc_dir / "zone_cert_key.pem")

    # Keep CA for trust
    copy_if_exists(ca_cert_path, etc_dir / "ca.cert")
    copy_if_exists(ca_key_path, etc_dir / "ca_key.pem")
    print(f"tls certs generated under {etc_dir}")

    post_gateway_config_str = f"""
stacks:
  zone_gateway_https:
    bind: 0.0.0.0:443
    protocol: tls
    certs:
      - domain: "{zone_id}"
        cert_path: ./zone_cert.cert
        key_path: ./zone_cert_key.pem
      - domain: "*.{zone_id}"
        cert_path: ./zone_cert.cert
        key_path: ./zone_cert_key.pem
    hook_point:
      main:
        id: main
        priority: 1
        blocks:
          default:
            id: default
            priority: 1
            block: |
              return "server node_gateway";
    """
    write_text(etc_dir / "post_gateway.yaml", post_gateway_config_str)


def make_identity_files(
    target_dir: Path,
    username: str,
    zone_id: str,
    node_name: str,
    netid: str,
    rtcp_port: int,
    sn_base_host: str,
    web3_bridge: str,
    ca_name: str,
    ca_dir: Optional[Path],
) -> None:
    """Use buckycli to generate identity files and use cert_mgr to generate TLS certificates."""
    if not BUCKYCLI_BIN.exists():
        raise FileNotFoundError(f"buckycli binary missing at {BUCKYCLI_BIN}")

    tmp_root = ensure_dir(BUCKYCLI_DIR)
    user_tmp = ensure_dir(tmp_root / zone_id)
    node_name_for_zone = node_name
    if netid != "lan":
        node_name_for_zone = f"{node_name}@{netid}"

    # Cases that need SN:
    # Has sn_base_host, node_name, netid is lan: standard node behind NAT
    # Has sn_base_host, node_name, netid is wan: need to configure ddns_sn_url
    # Has sn_base_host, node_name, netid is portmap: node with portmap enabled

    # 1. Create user/zone
    run_buckycli(
        [
            "create_user_env",
            "--username",
            username,
            "--hostname",
            zone_id,
            "--ood_name",
            node_name_for_zone,
            "--sn_base_host",
            sn_base_host,
            "--rtcp_port",
            str(rtcp_port),
            "--output_dir",
            str(user_tmp),
        ]
    )

    # 2. Create node configuration
    run_buckycli(
        [
            "create_node_configs",
            "--device_name",
            node_name,
            "--net_id",
            netid,
            "--env_dir",
            str(user_tmp),
        ]
    )

    # 3. Copy identity files
    user_dir = user_tmp
    node_dir = user_dir / node_name
    _copy_identity_outputs(user_dir, node_dir, target_dir, zone_id)

    # 4. TLS certificates
    _generate_tls(did_host_to_real_host(zone_id, web3_bridge), ca_name, ensure_dir(target_dir / "etc"), ca_dir)

    


def make_repo_cache_file(target_dir: Path) -> None:
    """Write meta_index cache file (placeholder to prevent auto-update)."""
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

def add_user_to_sn(root_dir: Path, username: str, sn_db_path: Path) -> None:
    """Add user to SN database."""
    run_buckycli(
        ["register_user_to_sn", "--username", username, "--sn_db_path", str(sn_db_path), "--output_dir", str(root_dir)]
    )
    print(f"root directory: {root_dir}")
    print(f"added user {username} to SN database at {sn_db_path}")


def add_device_to_sn(root_dir: Path, username: str, device_name: str, sn_db_path: Path) -> None:
    """Add device to SN database."""
    run_buckycli(
        ["register_device_to_sn", "--username", username, "--device_name", device_name, "--sn_db_path", str(sn_db_path), "--output_dir", str(root_dir)]
    )
    print(f"root directory: {root_dir}")
    print(f"added device {username}.{device_name} to SN database at {sn_db_path}")


def make_sn_configs(
    target_dir: Path,
    sn_base_host: str,
    sn_ip: str,
    sn_device_name: str = "sn_server",
    ca_name: str = "buckyos_test_ca",
    ca_dir: Path = None,
) -> None:
    """Generate SN (Super Node) server configuration files.
    
    All configuration files are placed directly in target_dir, including:
    - sn_server_private_key.pem - device private key file used by rtcp protocol stack
    - fullchain.cert, fullchain.pem - certificate and key containing sn.$sn_base, *.web3.$sn_base
    - ca/buckyos_sn_ca_cert.pem, ca/buckyos_sn_ca_key.pem - self-signed CA certificate for test environment
    - zone_zone - auto-generated, contains buckyos customized DNS TXT record template
    
    Note: The following files need to be manually created by users, not generated by this script:
    - dns_zone - manually configured DNS Zone file
    - website.yaml - website configuration file referenced by web3_gateway
    
    Args:
        target_dir: output directory, all files are placed directly in this directory
        sn_base_host: SN base domain (e.g. buckyos.io or devtests.org)
        sn_ip: SN server IP address
        sn_device_name: SN device name, default "sn_server"
        ca_name: CA certificate name
        ca_dir: use existing CA directory, otherwise auto-generate
    """
    if not BUCKYCLI_BIN.exists():
        raise FileNotFoundError(f"buckycli binary missing at {BUCKYCLI_BIN}")
    
    print(f"Generating SN configuration files to {target_dir} ...")
    print(f"  SN base domain: {sn_base_host}")
    print(f"  SN IP address: {sn_ip}")
    print(f"  SN device name: {sn_device_name}")
    
    # SN configuration files are placed directly in target_dir, no etc subdirectory created
    ensure_dir(target_dir)
    
    # 1. Use buckycli to create SN configuration
    # Note: SN uses special identity, using buckycli's create_sn_configs command here
    print("# Step 1: Create SN device identity configuration...")
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
    
    # buckycli generates files under target_dir/sn_server/, need to move to target_dir
    buckycli_sn_dir = target_dir / "sn_server"
    if buckycli_sn_dir.exists():
        # Move generated files to target_dir root directory
        for file in buckycli_sn_dir.glob("*"):
            if file.is_file():
                dest_file = target_dir / file.name
                shutil.move(str(file), str(dest_file))
                print(f"Move file: {file.name} -> {target_dir}/")
        # Remove empty sn_server directory
        if buckycli_sn_dir.exists() and not list(buckycli_sn_dir.iterdir()):
            buckycli_sn_dir.rmdir()

    
    # 2. Generate TLS certificates
    print("# Step 2: Generate TLS certificates...")

    cm = CertManager()
    
    ca_cert_path, ca_key_path = _check_or_generate_ca(cm, ca_name, ca_dir)
    
    # Generate server certificate (containing sn.$sn_base and *.web3.$sn_base)
    sn_hostname = f"sn.{sn_base_host}"
    web3_wildcard = f"*.web3.{sn_base_host}"
    
    cert_path, key_path = cm.create_cert_from_ca(
        str(ca_dir),
        hostname=sn_hostname,
        target_dir=str(target_dir),
        hostnames=[sn_hostname, web3_wildcard],
    )
    
    # Copy/rename to standard filenames
    cert_file = Path(cert_path)
    key_file = Path(key_path)
    
    shutil.move(cert_file, target_dir / "fullchain.cert")
    shutil.move(key_file, target_dir / "fullchain.pem")
    
    # Copy CA certificate to ca directory (for client trust)
    if ca_dir:
        ca_output_dir = ensure_dir(target_dir / "ca")
        shutil.copy2(ca_cert_path, ca_output_dir / ca_cert_path.name)
        shutil.copy2(ca_key_path, ca_output_dir / ca_key_path.name)
    
    print(f"TLS certificates generated:")
    print(f"  - {target_dir / 'fullchain.cert'}")
    print(f"  - {target_dir / 'fullchain.pem'}")
    print(f"  - {target_dir / 'ca' / ca_cert_path.name}")
    
    #3 Modify params.json
    params_json = json.load(open(target_dir / "params.json"))
    params_json["params"]["sn_ip"] = sn_ip
    write_json(target_dir / "params.json", params_json)
    
    print(f"\n[OK] SN configuration files generation completed!")
    print(f"  Output directory: {target_dir}")
    print(f"\nGenerated files:")
    print(f"  - {target_dir / 'sn_device_config.json'} (SN server device config)")
    print(f"  - {target_dir / 'sn_private_key.pem'} (device private key)")
    print(f"  - {target_dir / 'fullchain.cert'} (server certificate)")
    print(f"  - {target_dir / 'fullchain.pem'} (server private key)")
    print(f"  - {target_dir / 'ca' / 'buckyos_sn_ca_cert.pem'} (CA certificate)")
    print(f"  - {target_dir / 'params.json'} (SN configuration parameters)")
    print(f"\nFiles that need to be manually created:")
    print(f"  - {target_dir / 'dns_zone'} (DNS Zone configuration)")
    print(f"  - {target_dir / 'website.yaml'} (website configuration)")
    print(f"\nOther notes:")
    print(f"  - Test environment needs to install CA certificate to client trust list")


def make_sn_db(target_dir: Path, user_list: List[str]) -> None:
    """Placeholder, to be supplemented as needed."""
    print("skip sn_db generation (not implemented)")


def did_host_to_real_host(did_host: str,web3_bridge: str) -> str:
    """Convert DID hostname to real hostname."""
    if did_host.endswith(".bns.did"):
        result = did_host.split(".bns.did")[0] + "." + web3_bridge
        print(f"did_host_to_real_host: {did_host} -> {result}")
        return result
    return did_host

def get_params_from_group_name(group_name: str) -> Dict[str, object]:
    """Get all generation parameters based on group name."""
    if group_name == "dev" or group_name == "devtest_ood1":
        return {
            "username": "devtest",
            "zone_id": "test.buckyos.io",
            "node_name": "ood1",
            "netid": "wan",
            "rtcp_port": 2980,
            "sn_base_host": "",
            "web3_bridge": "web3.devtests.org",
            "trust_did": [
                "did:web:buckyos.org",
                "did:web:buckyos.ai",
                "did:web:buckyos.io",
            ],
            "force_https": False,
            "ca_name": "buckyos_test_ca",
            "is_sn": False,
        }
    if group_name == "alice.ood1":
        return {
            "username": "alice",
            "zone_id": "alice.bns.did",
            "node_name": "ood1",
            "netid": "lan",
            "rtcp_port": 2980,
            "sn_base_host": "devtests.org",
            "web3_bridge": "web3.devtests.org",
            "trust_did": [
                "did:web:buckyos.org",
                "did:web:buckyos.ai",
                "did:web:buckyos.io",
            ],
            "force_https": False,
            "ca_name": "buckyos_test_ca",
            "is_sn": False,
        }
    if group_name == "bob.ood1":
        return {
            "username": "bob",
            "zone_id": "bob.bns.did",
            "node_name": "ood1",
            "netid": "wan_dyn",
            "rtcp_port": 2980,
            "sn_base_host": "devtests.org", # netid is wan but has SN, means need to use d-dns
            "web3_bridge": "web3.devtests.org",
            "ddns_sn_url": f"https://sn.devtests.org/kapi/sn",
            "trust_did": [
                "did:web:buckyos.org",
                "did:web:buckyos.ai",
                "did:web:buckyos.io",
            ],
            "force_https": False,
            "ca_name": "buckyos_test_ca",
            "is_sn": False,
        }
    if group_name == "charlie.ood1":
        return {
            "username": "charlie",
            "zone_id": "charlie.me",
            "node_name": "ood1",
            "netid": "portmap", #portmap https goes through relay, rtcp can connect directly
            "rtcp_port": 2981, # using custom rtcp port
            "sn_base_host": "devtests.org",
            "web3_bridge": "web3.devtests.org",
            "trust_did": [
                "did:web:buckyos.org",
                "did:web:buckyos.ai",
                "did:web:buckyos.io",
            ],
            "force_https": False,
            "ca_name": "buckyos_test_ca",
            "is_sn": False,
        }
    if group_name == "sn_server" or group_name == "sn":
        return {
            "sn_base_host": "devtests.org",
            "sn_ip": "192.168.64.84", #TODO: need to get from external (environment variable is simplest?)
            "sn_device_name": "sn_server", 
            "web3_bridge": "web3.devtests.org",
            "trust_did": [
                "did:web:buckyos.org",
                "did:web:buckyos.ai",
                "did:web:buckyos.io",
            ],
            "force_https": False,
            "ca_name": "buckyos_test_ca",
            "is_sn": True,
        }
    if group_name == "devtests_ood1" or group_name == "sn_web":
        return {
            "username": "devtests",
            "zone_id": "devtests.org",
            "node_name": "ood1",
            "netid": "wan", 
            "rtcp_port": 2980,
            "sn_base_host": "",
            "web3_bridge": "web3.devtests.org",
            "trust_did": [
                "did:web:buckyos.org",
                "did:web:buckyos.ai",
                "did:web:buckyos.io",
            ],
            "force_https": False,
            "ca_name": "buckyos_test_ca",
            "is_sn": False,
        }
    raise ValueError(f"invalid group name: {group_name}")

def get_local_ip() -> str:
    """Get local machine IP address."""
    # Get local IP
    import socket
    try:
        # Get local IP by connecting to external address (recommended method)
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        s.connect(("8.8.8.8", 80))
        ip_address = s.getsockname()[0]
        s.close()
    except Exception:
        # Fall back to hostname method
        hostname = socket.gethostname()
        ip_address = socket.gethostbyname(hostname)
            
    print(f"ip_address: {ip_address}")
    return ip_address

def make_config_by_group_name(group_name: str, target_root: Optional[Path], ca_dir: Optional[Path],sn_ip: Optional[str],env_root: Optional[Path]) -> None:
    params = get_params_from_group_name(group_name)
    if ca_dir is None:
        ca_dir = ensure_dir(BUCKYCLI_DIR / "ca")
    print(f"############ make config for group name: {group_name} #########################")
    print(f"rootfs dir : {target_root}")
    print(f"group      : {group_name}")
    
    is_sn = params.get("is_sn", False)
    
    if is_sn:
        if target_root is None:
            target_root = Path("/opt/web3-gateway")

        if env_root is None:
            env_root = BUCKYCLI_DIR

        if sn_ip is None:
            sn_ip = params.get("sn_ip", None)
            if sn_ip is None:
                sn_ip = get_local_ip()

        # SN configuration generation
        print(f"sn_base_host: {params['sn_base_host']}")
        print(f"sn_ip       : {sn_ip}")
        print(f"device_name : {params['sn_device_name']}")
        print(f"web3_bridge : {params['web3_bridge']}")
        
        # SN doesn't need machine.json, did_docs cache and meta_index cache
        make_sn_configs(
            target_root,
            params["sn_base_host"],
            sn_ip,
            params["sn_device_name"],
            params["ca_name"],
            ca_dir,
        )

        # Add default users and devices to SN database
        db_path = target_root / "sn_db.sqlite3"
        # alice.ood1
        add_user_to_sn(env_root, "alice.web3.devtests.org", db_path)
        add_device_to_sn(env_root, "alice.web3.devtests.org", "ood1", db_path)

        # bob.ood1
        add_user_to_sn(env_root, "bob.web3.devtests.org", db_path)
        add_device_to_sn(env_root, "bob.web3.devtests.org", "ood1", db_path)

        #charlie.ood1
        add_user_to_sn(env_root, "charlie.me", db_path)
        add_device_to_sn(env_root, "charlie.me", "ood1", db_path)
    else:
        if target_root is None:
            target_root = ROOTFS_DIR
        # Normal OOD node configuration generation
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
            params["netid"],
            params["rtcp_port"],
            params["sn_base_host"],
            params["web3_bridge"],
            params["ca_name"],
            ca_dir,
        )
        make_repo_cache_file(target_root)
    
    print(f"config {group_name} generation finished.")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate configuration files under rootfs",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    parser.add_argument("group", help="Configuration group name, e.g. dev")
    parser.add_argument(
        "--rootfs",
        default=None,
        type=Path,
        help="Output directory",
    )
    parser.add_argument(
        "--ca",
        default=None,
        type=Path,
        help="Use existing CA directory (with *_ca_cert.pem and corresponding key), otherwise auto-generate",
    )
    parser.add_argument(
        "--sn_ip",
        default=None,
        type=str,
        help="SN IP address",
    )
    args = parser.parse_args()
    make_config_by_group_name(args.group, args.rootfs, args.ca, args.sn_ip, None)

if __name__ == "__main__":
    sys.exit(main())

