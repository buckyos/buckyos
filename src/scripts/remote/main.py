import argparse
import json
import os
from pathlib import Path
import sys

from worksapce import Workspace


def build_parser() -> argparse.ArgumentParser:
    """Build an argparse parser with modular subcommands."""
    parser = argparse.ArgumentParser(
        description="Manage remote VMs and apps for a workspace group."
    )
    parser.add_argument("group_name", help="Workspace group name.")

    subparsers = parser.add_subparsers(dest="command", required=True)

    clean_parser = subparsers.add_parser(
        "clean_vms", help="Remove all Multipass instances for this group."
    )
    clean_parser.add_argument(
        "--force",
        action="store_true",
        help="Skip confirmation prompts (behavior depends on implementation).",
    )
    clean_parser.set_defaults(handler=handle_clean_vms)

    create_parser = subparsers.add_parser(
        "create_vms", help="Create VMs from workspace configuration."
    )
    create_parser.set_defaults(handler=handle_create_vms)

    snapshot_parser = subparsers.add_parser(
        "snapshot", help="Create snapshots for all VMs."
    )
    snapshot_parser.add_argument("snapshot_name", help="Snapshot name.")
    snapshot_parser.set_defaults(handler=handle_snapshot)

    restore_parser = subparsers.add_parser(
        "restore", help="Restore snapshots for all VMs."
    )
    restore_parser.add_argument("snapshot_name", help="Snapshot name.")
    restore_parser.set_defaults(handler=handle_restore)

    info_parser = subparsers.add_parser(
        "info_vms", help="Show VM status information."
    )
    info_parser.set_defaults(handler=handle_info_vms)

    install_parser = subparsers.add_parser(
        "install", help="Install apps to a device based on configuration."
    )
    install_parser.add_argument("device_id", help="Target device id.")
    install_parser.add_argument(
        "--apps",
        nargs="+",
        help="Specify app names to install; defaults to all configured apps.",
    )
    install_parser.set_defaults(handler=handle_install)

    update_parser = subparsers.add_parser(
        "update", help="Update apps on a device based on configuration."
    )
    update_parser.add_argument("device_id", help="Target device id.")
    update_parser.add_argument(
        "--apps",
        nargs="+",
        help="Specify app names to update; defaults to all configured apps.",
    )
    update_parser.set_defaults(handler=handle_update)

    start_parser = subparsers.add_parser(
        "start", help="Start buckyos on all VMs (SN not started)."
    )
    start_parser.set_defaults(handler=handle_start)

    stop_parser = subparsers.add_parser(
        "stop", help="Stop buckyos on all VMs."
    )
    stop_parser.set_defaults(handler=handle_stop)

    clog_parser = subparsers.add_parser(
        "clog", help="Collect logs from nodes."
    )
    clog_parser.set_defaults(handler=handle_clog)

    run_parser = subparsers.add_parser(
        "run", help="Execute commands on a specific node."
    )
    run_parser.add_argument("node_id", help="Target node id.")
    run_parser.add_argument(
        "cmds",
        nargs="+",
        help="Command(s) to execute; provide multiple to run sequentially.",
    )
    run_parser.set_defaults(handler=handle_run)

    return parser


def build_workspace(group_name: str) -> Workspace:
    """Create and load a workspace instance."""
    workspace_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "dev_configs", group_name)
    print(f"{group_name} workspace_dir: {workspace_dir}")
    workspace = Workspace(Path(workspace_dir))
    workspace.load()
    return workspace


def handle_clean_vms(workspace: Workspace, args: argparse.Namespace) -> None:
    workspace.clean_vms()


def handle_create_vms(workspace: Workspace, args: argparse.Namespace) -> None:
    workspace.create_vms()


def handle_snapshot(workspace: Workspace, args: argparse.Namespace) -> None:
    workspace.snapshot(args.snapshot_name)


def handle_restore(workspace: Workspace, args: argparse.Namespace) -> None:
    workspace.restore(args.snapshot_name)


def handle_info_vms(workspace: Workspace, args: argparse.Namespace) -> None:
    info = workspace.info_vms()
    if info is not None:
        print(json.dumps(info, indent=2, ensure_ascii=False))


def handle_install(workspace: Workspace, args: argparse.Namespace) -> None:
    print(f"install apps to device: {args.device_id} with apps: {args.apps}")
    workspace.install(args.device_id, args.apps)


def handle_update(workspace: Workspace, args: argparse.Namespace) -> None:
    workspace.update(args.device_id, args.apps)


def handle_start(workspace: Workspace, args: argparse.Namespace) -> None:
    workspace.start()


def handle_stop(workspace: Workspace, args: argparse.Namespace) -> None:
    workspace.stop()


def handle_clog(workspace: Workspace, args: argparse.Namespace) -> None:
    workspace.clog()


def handle_run(workspace: Workspace, args: argparse.Namespace) -> None:
    workspace.run(args.node_id, args.cmds)


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    workspace = build_workspace(args.group_name)
    handler = getattr(args, "handler", None)
    if handler is None:
        parser.print_help()
        return 1

    handler(workspace, args)
    return 0


if __name__ == "__main__":
    sys.exit(main())