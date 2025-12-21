#!/usr/bin/env python3
"""
Modify the system hosts file to set sn.devtests.org to the specified IP address.
Usage: python set_host.py <ip_address>
Example: python set_host.py 192.168.1.100
Note: Duplicate entries will not be added.
"""

import argparse
import platform
import sys
from pathlib import Path


def get_hosts_file_path() -> Path:
    """Get the path to the hosts file based on the operating system."""
    system = platform.system()
    if system == "Windows":
        return Path("C:/Windows/System32/drivers/etc/hosts")
    else:
        # Linux and macOS
        return Path("/etc/hosts")


def read_hosts_file(hosts_path: Path) -> list[str]:
    """Read the hosts file and return all lines."""
    try:
        with open(hosts_path, "r", encoding="utf-8") as f:
            return f.readlines()
    except PermissionError:
        print(f"Error: Permission denied. Please run with administrator/sudo privileges.")
        sys.exit(1)
    except FileNotFoundError:
        print(f"Error: Hosts file not found at {hosts_path}")
        sys.exit(1)
    except Exception as e:
        print(f"Error reading hosts file: {e}")
        sys.exit(1)


def write_hosts_file(hosts_path: Path, lines: list[str]) -> None:
    """Write lines back to the hosts file."""
    try:
        with open(hosts_path, "w", encoding="utf-8") as f:
            f.writelines(lines)
    except PermissionError:
        print(f"Error: Permission denied. Please run with administrator/sudo privileges.")
        sys.exit(1)
    except Exception as e:
        print(f"Error writing hosts file: {e}")
        sys.exit(1)


def update_hosts_entry(ip_address: str, hostname: str = "sn.devtests.org") -> None:
    """
    Update or add the hosts file entry for the specified hostname.
    
    Args:
        ip_address: The IP address to set
        hostname: The hostname to map (default: sn.devtests.org)
    """
    hosts_path = get_hosts_file_path()
    
    if not hosts_path.exists():
        print(f"Error: Hosts file not found at {hosts_path}")
        sys.exit(1)
    
    # Read existing hosts file
    lines = read_hosts_file(hosts_path)
    
    # Track if we found and updated the entry
    found = False
    new_lines = []
    
    for line in lines:
        stripped = line.strip()
        
        # Skip empty lines and comments
        if not stripped or stripped.startswith("#"):
            new_lines.append(line)
            continue
        
        # Parse the line: IP address followed by hostnames
        parts = stripped.split()
        if len(parts) < 2:
            new_lines.append(line)
            continue
        
        line_ip = parts[0]
        line_hostnames = parts[1:]
        
        # Check if this line contains our hostname
        if hostname in line_hostnames:
            found = True
            # If IP is different, update the line
            if line_ip != ip_address:
                # Remove the hostname from existing hostnames
                remaining_hostnames = [h for h in line_hostnames if h != hostname]
                
                # If there are other hostnames, keep the line with them
                if remaining_hostnames:
                    new_lines.append(f"{line_ip} {' '.join(remaining_hostnames)}\n")
                # Otherwise, skip this line (we'll add a new one)
            else:
                # IP is already correct, keep the line as is
                new_lines.append(line)
        else:
            # This line doesn't contain our hostname, keep it
            new_lines.append(line)
    
    # If not found, add a new entry
    if not found:
        new_lines.append(f"{ip_address} {hostname}\n")
        print(f"Added: {ip_address} {hostname}")
    else:
        # Check if we need to add a new entry (if the old one was removed)
        has_entry = any(hostname in line and ip_address in line for line in new_lines)
        if not has_entry:
            new_lines.append(f"{ip_address} {hostname}\n")
            print(f"Updated: {ip_address} {hostname}")
        else:
            print(f"Entry already exists: {ip_address} {hostname}")
    
    # Write back to hosts file
    write_hosts_file(hosts_path, new_lines)
    print(f"Hosts file updated successfully: {hosts_path}")


def validate_ip_address(ip: str) -> bool:
    """Validate IP address format."""
    parts = ip.split(".")
    if len(parts) != 4:
        return False
    try:
        for part in parts:
            num = int(part)
            if num < 0 or num > 255:
                return False
        return True
    except ValueError:
        return False


def main():
    """Main entry point."""
    parser = argparse.ArgumentParser(
        description="Set sn.devtests.org to the specified IP address in the hosts file"
    )
    parser.add_argument(
        "ip_address",
        type=str,
        help="IP address to set (e.g., 192.168.1.100)"
    )
    
    args = parser.parse_args()
    
    # Validate IP address
    if not validate_ip_address(args.ip_address):
        print(f"Error: Invalid IP address format: {args.ip_address}")
        print("Expected format: xxx.xxx.xxx.xxx (e.g., 192.168.1.100)")
        sys.exit(1)
    
    # Update hosts file
    update_hosts_entry(args.ip_address)


if __name__ == "__main__":
    main()
