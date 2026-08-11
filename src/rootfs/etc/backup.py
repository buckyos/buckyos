#!/usr/bin/env python3
# backup and restore identity file
# backup.py $target_dir
#    Find etc/node_identity.json file under buckyos_rootfs_dir
#    Backup files based on node_identity.json content to target_dir (default target_dir is $HOME/buckyos_backup)
# restore.py $target_dir $zone_name
#    Find corresponding directory under target_dir by zone_name and restore files

import os
import sys
import json
import shutil
import argparse
from pathlib import Path


def get_buckyos_root():
    """Get buckyos root directory"""
    buckyos_root = os.environ.get("BUCKYOS_ROOT")
    if buckyos_root:
        return buckyos_root
    
    if sys.platform == "win32":
        user_data_dir = os.environ.get("APPDATA")
        if not user_data_dir:
            user_data_dir = os.environ.get("USERPROFILE", ".")
        return os.path.join(user_data_dir, "buckyos")
    else:
        return "/opt/buckyos"

def get_backup_file_list():
    """Get backup file list"""
    return [
        "node_identity.json",
        "start_config.json",
        "machine.json"
    ]

def get_buckyos_rootfs_dir():
    """Get buckyos rootfs directory (i.e., buckyos root directory)"""
    return get_buckyos_root()


def sanitize_filename(name: str) -> str:
    """Sanitize filename for Windows compatibility (remove invalid characters)"""
    invalid_chars = '<>:"|?*'
    for char in invalid_chars:
        name = name.replace(char, '_')
    return name


def get_identity_root(buckyos_root: str) -> Path:
    return Path(os.environ.get("BUCKYOS_IDENTITY_ROOT", Path(buckyos_root) / "local" / "identity"))


def get_security_root(buckyos_root: str) -> Path:
    return Path(os.environ.get("BUCKYOS_SECURITY_ROOT", Path(buckyos_root) / "security"))


def find_device_identity_dir(buckyos_root: str, device_did: str):
    if not device_did:
        return None

    identity_root = get_identity_root(buckyos_root)
    if not identity_root.exists():
        return None

    for did_json in identity_root.glob("*/did.json"):
        try:
            with open(did_json, "r", encoding="utf-8") as f:
                did_doc = json.load(f)
        except Exception:
            continue
        if did_doc.get("id") == device_did:
            return did_json.parent

    return None


def copy_file(src_file: Path, dst_file: Path, label: str, copied_items: list):
    if src_file.exists():
        try:
            dst_file.parent.mkdir(parents=True, exist_ok=True)
            print(f"Copying {src_file} -> {dst_file} ...")
            shutil.copy2(src_file, dst_file)
            copied_items.append(label)
            print(f"  ✓ Copied: {label}")
        except Exception as e:
            print(f"  ✗ Failed to copy {label}: {e}")
    else:
        print(f"  - Skipped (not found): {label}")


def copy_tree(src_dir: Path, dst_dir: Path, label: str, copied_items: list):
    if src_dir.exists():
        try:
            print(f"Copying {src_dir} -> {dst_dir} ...")
            shutil.copytree(src_dir, dst_dir, dirs_exist_ok=True)
            copied_items.append(label)
            print(f"  ✓ Copied: {label}")
        except Exception as e:
            print(f"  ✗ Failed to copy {label}: {e}")
    else:
        print(f"  - Skipped (not found): {label}")


def backup_identity_file(target_dir: str):
    """
    Backup identity files
    Files to backup: [etc/node_identity.json, etc/start_config.json, etc/machine.json, local/identity/<device>, security/<device>]
    Backup to target_dir/$zone_name directory, zone_name is obtained from start_config.json file
    """
    buckyos_root = get_buckyos_rootfs_dir()
    etc_dir = Path(buckyos_root) / "etc"
    start_config_file = etc_dir / "start_config.json"
    
    # Check if start_config.json exists
    if not start_config_file.exists():
        print(f"Error: {start_config_file} not found!")
        sys.exit(1)
    
    # Read start_config.json to get zone_name
    try:
        with open(start_config_file, 'r', encoding='utf-8') as f:
            start_config_data = json.load(f)
    except json.JSONDecodeError as e:
        print(f"Error: Failed to parse {start_config_file}: {e}")
        sys.exit(1)
    except Exception as e:
        print(f"Error: Failed to read {start_config_file}: {e}")
        sys.exit(1)
    
    # Get zone_name
    zone_name = start_config_data.get("zone_name")
    if not zone_name:
        print(f"Error: zone_name not found in {start_config_file}")
        sys.exit(1)
    
    # Sanitize zone_name for Windows compatibility
    zone_name_safe = sanitize_filename(str(zone_name))
    
    # Create backup target directory
    backup_base_dir = Path(target_dir).expanduser()
    backup_dir = backup_base_dir / zone_name_safe
    backup_dir.mkdir(parents=True, exist_ok=True)
    
    print(f"Backing up identity files to: {backup_dir}")
    print(f"Zone name: {zone_name}")
    
    # List of files to backup
    files_to_backup = get_backup_file_list()
    
    # Backup files
    backed_up_files = []
    for filename in files_to_backup:
        src_file = etc_dir / filename
        dst_file = backup_dir / "etc" / filename
        copy_file(src_file, dst_file, f"etc/{filename}", backed_up_files)

    node_identity_file = etc_dir / "node_identity.json"
    if node_identity_file.exists():
        try:
            with open(node_identity_file, "r", encoding="utf-8") as f:
                node_identity = json.load(f)
            device_did = node_identity.get("device_did", "")
            identity_dir = find_device_identity_dir(buckyos_root, device_did)
            if identity_dir:
                dir_name = identity_dir.name
                security_dir = get_security_root(buckyos_root) / dir_name
                copy_tree(
                    identity_dir,
                    backup_dir / "local" / "identity" / dir_name,
                    f"local/identity/{dir_name}",
                    backed_up_files,
                )
                copy_tree(
                    security_dir,
                    backup_dir / "security" / dir_name,
                    f"security/{dir_name}",
                    backed_up_files,
                )
            else:
                print(f"  - Skipped identity directories (not found for {device_did})")
        except Exception as e:
            print(f"  ✗ Failed to backup identity directories: {e}")
    
    if backed_up_files:
        print(f"\nBackup completed! {len(backed_up_files)} file(s) backed up to {backup_dir}")
    else:
        print("\nWarning: No files were backed up!")
        sys.exit(1)


def restore_identity_file(source_dir: str, zone_name: str):
    """
    Restore identity files
    Find corresponding directory under target_dir by zone_name and restore files
    """
    buckyos_root = get_buckyos_rootfs_dir()
    etc_dir = Path(buckyos_root) / "etc"
    
    # Sanitize zone_name for Windows compatibility
    zone_name_safe = sanitize_filename(zone_name)
    
    # Find backup directory
    backup_base_dir = Path(source_dir).expanduser()
    backup_dir = backup_base_dir / zone_name_safe
    
    if not backup_dir.exists():
        print(f"Error: Backup directory not found: {backup_dir}")
        sys.exit(1)
    
    if not backup_dir.is_dir():
        print(f"Error: {backup_dir} is not a directory")
        sys.exit(1)
    
    print(f"Restoring identity files from: {backup_dir}")
    print(f"Zone name: {zone_name}")
    
    # Ensure etc directory exists
    etc_dir.mkdir(parents=True, exist_ok=True)
    
    restored_files = []
    for filename in get_backup_file_list():
        src_file = backup_dir / "etc" / filename
        dst_file = etc_dir / filename
        copy_file(src_file, dst_file, f"etc/{filename}", restored_files)

    identity_backup_root = backup_dir / "local" / "identity"
    if identity_backup_root.exists():
        for identity_dir in identity_backup_root.iterdir():
            if identity_dir.is_dir():
                copy_tree(
                    identity_dir,
                    get_identity_root(buckyos_root) / identity_dir.name,
                    f"local/identity/{identity_dir.name}",
                    restored_files,
                )
    else:
        print(f"  - Skipped (not found in backup): local/identity")

    security_backup_root = backup_dir / "security"
    if security_backup_root.exists():
        for security_dir in security_backup_root.iterdir():
            if security_dir.is_dir():
                copy_tree(
                    security_dir,
                    get_security_root(buckyos_root) / security_dir.name,
                    f"security/{security_dir.name}",
                    restored_files,
                )
    else:
        print(f"  - Skipped (not found in backup): security")
    
    if restored_files:
        print(f"\nRestore completed! {len(restored_files)} file(s) restored to {etc_dir}")
    else:
        print("\nWarning: No files were restored!")
        sys.exit(1)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Backup and restore buckyos identity files")
    subparsers = parser.add_subparsers(dest="command", help="Command to execute")
    
    # backup command
    backup_parser = subparsers.add_parser("backup", help="Backup identity files")
    backup_parser.add_argument(
        "target_dir",
        nargs="?",
        default=None,
        help="Target backup directory (default: ~/buckyos_backup)"
    )
    
    # restore command
    restore_parser = subparsers.add_parser("restore", help="Restore identity files")
    restore_parser.add_argument(
        "zone_name",
        help="Zone name to restore (e.g., test.buckyos.io)"
    )
    restore_parser.add_argument(
        "source_dir",
        nargs="?",
        default=None,
        help="Backup directory where zone_name is located"
    )

    
    args = parser.parse_args()
    
    if args.command == "backup":
        target_dir = args.target_dir if args.target_dir else os.path.expanduser("~/buckyos_backup")
        backup_identity_file(target_dir)
    elif args.command == "restore":
        source_dir = args.source_dir if args.source_dir else os.path.expanduser("~/buckyos_backup")
        restore_identity_file(source_dir, args.zone_name)
    else:
        parser.print_help()
        sys.exit(1)
