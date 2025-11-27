"""Generate dev test configs under dev_configs using cert_mgr and buckycli.

Features:
  - Create CA and self-signed certificates for SN / web3 domains.
  - Create users (with zone): bob, alice.
  - Create device configs based on users: bob.ood1, alice.ood1, sn_server.
  - (Reserved) Update SN database to add bob / alice.

Dependencies:
  - Built `buckycli` binary, path via env `BUCKYCLI_BIN` (default: `buckycli`).
  - OpenSSL CLI, used by `cert_mgr.CertManager`.
"""

import json
import os
import shutil
import subprocess
from pathlib import Path
from typing import List, Dict, Optional

from py_src import cert_mgr
from py_src import util


# =============================================================================
# Global settings
# =============================================================================

BASE_DIR = Path(os.path.dirname(os.path.abspath(__file__)))
ROOTFS_DIR = BASE_DIR.parent.parent / "rootfs"
CONFIG_BASE = Path(util.CONFIG_BASE)  # usually src/scripts/remote/dev_configs

# Default domains and users, adjust as needed
BASE_SUPER_DOMAIN = "devtests.org"
SN_HOST = f"sn.{BASE_SUPER_DOMAIN}"
WEB3_ROOT = f"web3.{BASE_SUPER_DOMAIN}"

MACHINE_JSON_CONTENT = {
    "web3_bridge": {
        "bns": WEB3_ROOT
    },
    "trust_did": [
        "did:web:buckyos.org",
        "did:web:buckyos.ai",
        "did:web:buckyos.io",
        f"did:web:{BASE_SUPER_DOMAIN}",
    ]
}

USERS: List[Dict[str, object]] = [
    {
        "user_id": "bob",          # subdir under owners/
        "username": "bob",         # buckycli username
        "zone_hostname": f"bob.{WEB3_ROOT}",
        "netid": "lan2",
        "devices": ["ood1"],
    },
    {
        "user_id": "alice",
        "username": "alice",
        "zone_hostname": f"alice.{WEB3_ROOT}",
        "netid": "lan3",
        "devices": ["ood1"],
    },
]

SN_DEVICE: Dict[str, object] = {
    "user_id": "sn",
    "username": "sn",
    "zone_hostname": WEB3_ROOT,
    "devices": ["sn_server"],
}

BUCKYCLI_BIN = ROOTFS_DIR / "bin" / "buckycli" / "buckycli"


# =============================================================================
# Helpers
# =============================================================================

def run_cmd(cmd: List[str], cwd: Optional[Path] = None) -> None:
    """Run a subprocess command, printing output and raising on failure."""
    #print(f"[RUN] {' '.join(cmd)} (cwd={cwd or os.getcwd()})")
    result = subprocess.run(
        cmd,
        cwd=str(cwd) if cwd is not None else None,
        text=True,
        capture_output=True,
    )
    if result.stdout:
        print(result.stdout)
    if result.stderr:
        print(result.stderr)
    if result.returncode != 0:
        raise RuntimeError(f"command failed: {' '.join(cmd)}")


def run_buckycli(args: List[str]) -> None:
    """Call buckycli subcommand from repo root."""
    cmd = [BUCKYCLI_BIN] + args
    run_cmd(cmd, cwd=CONFIG_BASE)


def ensure_dir(path: Path) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    return path


# =============================================================================
# Certificates: CA + SN / web3 certs
# =============================================================================

def generate_ca_and_sn_certs() -> None:
    """Generate test CA and SN / web3 certificates using cert_mgr."""
    ca_dir = ensure_dir(CONFIG_BASE)
    sn_cert_dir = ensure_dir(CONFIG_BASE / "sn_server" / "certs")

    cm = cert_mgr.CertManager()

    # 1. Create test CA
    print("=== Create test CA ===")
    cm.create_ca(str(ca_dir), name="buckyos_dev")

    # 2. Create certificates for SN and web3 domains
    print("=== Create SN / web3 certificates ===")
    cm.create_cert_from_ca(str(ca_dir), hostname=SN_HOST, target_dir=str(sn_cert_dir))
    cm.create_cert_from_ca(
        str(ca_dir),
        hostname=f"*.{WEB3_ROOT}",
        target_dir=str(sn_cert_dir),
    )




# =============================================================================
# Use buckycli to build user / device configs
# =============================================================================

def generate_user_env(user_cfg: Dict[str, object], tmp_root: Path) -> None:
    """Generate owner + zone configs for a single user using buckycli."""
    user_id = str(user_cfg["user_id"])
    username = str(user_cfg["username"])
    zone_hostname = str(user_cfg["zone_hostname"])
    netid = str(user_cfg["netid"])

    tmp_output = ensure_dir(tmp_root / user_id)

    # buckycli will create a DevEnvBuilder-style tree under tmp_output
    run_buckycli(
        [
            "create_user_env",
            "--username",
            username,
            "--hostname",
            zone_hostname,
            "--netid",
            netid,
            "--output_dir",
            str(tmp_output),
        ]
    )

    # buckycli creates a `username` subdir inside output_dir
    user_dir = tmp_output / username
    if not user_dir.exists():
        raise RuntimeError(f"expected user dir not found: {user_dir}")

    # Target owners dir: dev_configs/owners/{user_id}
    owners_dir = ensure_dir(CONFIG_BASE / "owners" / user_id)

    # Copy owner-related files into owners dir (flatten)
    for name in ("user_config.json", "user_private_key.pem"):
        src = user_dir / name
        if src.exists():
            dst = owners_dir / name
            print(f"copy {src} -> {dst}")
            shutil.copy2(src, dst)

    # Copy zone config JSON as zone_config.json
    # create_user_env creates {hostname}.zone.json
    zone_json = user_dir / f"{zone_hostname}.zone.json"
    if zone_json.exists():
        dst = owners_dir / "zone_config.json"
        print(f"copy {zone_json} -> {dst}")
        shutil.copy2(zone_json, dst)
    else:
        print(f"warning: zone json not found: {zone_json}")


def generate_user_devices(user_cfg: Dict[str, object], tmp_root: Path) -> None:
    """Generate device configs for a user and copy them under dev_configs root."""
    user_id = str(user_cfg["user_id"])
    username = str(user_cfg["username"])
    zone_hostname = str(user_cfg["zone_hostname"])
    devices = list(user_cfg.get("devices", []))

    tmp_output = ensure_dir(tmp_root / user_id)

    for dev_name in devices:
        dev_name = str(dev_name)

        # Generate device config via buckycli
        run_buckycli(
            [
                "create_node_configs",
                "--username",
                username,
                "--device_name",
                dev_name,
                "--zone_name",
                zone_hostname,
                "--output_dir",
                str(tmp_output),
                "--net_id",
                "lan1",
            ]
        )

        user_dir = tmp_output / username
        dev_dir = user_dir / dev_name
        if not dev_dir.exists():
            raise RuntimeError(f"expected device dir not found: {dev_dir}")

        # Target node dir: dev_configs/{user_id}.{dev_name}
        node_dir = ensure_dir(CONFIG_BASE / f"{user_id}.{dev_name}")

        # create machine.json
        machine_json = node_dir / "machine.json"
        with open(machine_json, "w") as f:
            json.dump(MACHINE_JSON_CONTENT, f)

        # Key files we care about
        for name in ("node_identity.json", "node_private_key.pem", "start_config.json"):
            src = dev_dir / name
            if src.exists():
                dst = node_dir / name
                print(f"copy {src} -> {dst}")
                shutil.copy2(src, dst)
            else:
                print(f"warning: device file not found: {src}")


def generate_sn_configs(tmp_root: Path) -> None:
    """Generate SN configs (sn_server directory), excluding DB details."""
    tmp_output = ensure_dir(tmp_root / "sn")

    # Use buckycli to create SN zone_boot_config etc.
    run_buckycli(
        [
            "create_sn_configs",
            "--output_dir",
            str(tmp_output),
        ]
    )

    # create_sn_config writes to output_dir/sn_server
    src_sn_dir = tmp_output / "sn_server"
    if not src_sn_dir.exists():
        print(f"warning: sn_server configs not found at {src_sn_dir}")
        return

    dst_sn_dir = ensure_dir(CONFIG_BASE / "sn_server")

    # Copy all generated SN configs (do not overwrite existing sn_db.sqlite3)
    for item in src_sn_dir.iterdir():
        if item.name == "sn_db.sqlite3" and (dst_sn_dir / item.name).exists():
            print(f"skip existing SN db: {dst_sn_dir / item.name}")
            continue
        dst = dst_sn_dir / item.name
        if item.is_file():
            print(f"copy {item} -> {dst}")
            shutil.copy2(item, dst)


# =============================================================================
# SN DB update (placeholder)
# =============================================================================

def update_sn_db_for_users() -> None:
    """Update SN DB to add bob / alice devices.

    Uses buckycli register_device command to register devices to SN database.
    """
    sn_db_path = CONFIG_BASE / "sn_server" / "sn_db.sqlite3"
    if not sn_db_path.exists():
        print(f"SN db not found, skip update: {sn_db_path}")
        return
    
    print("=== Register devices to SN database ===")
    
    # Register devices for each user
    for user in USERS:
        user_id = str(user["user_id"])
        username = str(user["username"])
        devices = list(user.get("devices", []))
        
        for dev_name in devices:
            dev_name = str(dev_name)
            print(f"Registering device {username}.{dev_name} to SN...")
            
            # Use buckycli to register device
            # Device configs should be in the tmp_root directory structure
            tmp_root = CONFIG_BASE / "_buckycli_tmp"
            run_buckycli(
                [
                    "register_device",
                    "--username",
                    username,
                    "--device_name",
                    dev_name,
                    "--sn_db_path",
                    str(sn_db_path),
                    "--output_dir",
                    str(tmp_root / user_id),
                ]
            )
    
    print("All devices registered to SN database.")


# =============================================================================
# Entry
# =============================================================================

def main() -> None:
    print(f"CONFIG_BASE = {CONFIG_BASE}")
    ensure_dir(CONFIG_BASE)

    tmp_root = ensure_dir(CONFIG_BASE / "_buckycli_tmp")

    # 1. CA + SN certs
    generate_ca_and_sn_certs()

    # 2. User (owner + zone) configs
    for user in USERS:
        generate_user_env(user, tmp_root)

    # 3. User device configs
    for user in USERS:
        generate_user_devices(user, tmp_root)

    # 4. SN configs
    generate_sn_configs(tmp_root)

    # 5. (optional) SN DB updates
    update_sn_db_for_users()

    print("All configs generated under dev_configs.")


if __name__ == "__main__":
    main()


