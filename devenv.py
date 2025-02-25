#!/usr/bin/env python3
import os
import subprocess
import sys

def check_package_manager():
    """Check if apt package manager is available"""
    result = subprocess.run(['which', 'apt-get'], capture_output=True)
    if result.returncode != 0:
        print("This script only supports Debian/Ubuntu")
        sys.exit(1)

def run_command(command, exit_on_error=True):
    """Run shell command and print output"""
    print(f"Executing command: {command}")
    result = subprocess.run(command, shell=True)
    if result.returncode != 0 and exit_on_error:
        print(f"Command execution failed: {command}")
        sys.exit(1)
    return result.returncode

def install_rust_toolchain():
    """Install Rust toolchain and cross-compilation support"""
    # Install basic development tools and OpenSSL dev library
    run_command("apt-get update")
    run_command("apt-get install -y build-essential curl wget git pkg-config libssl-dev docker.io")
    
    # Install Rust using rustup (instead of apt)
    run_command('curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y')
    
    # Reload environment variables to make rustup available
    run_command('source $HOME/.cargo/env', exit_on_error=False)
    
    # Install cross-compilation tools
    run_command("apt-get install -y musl-tools")
    run_command("apt-get install -y gcc-aarch64-linux-gnu")
    
    # Add Rust targets
    run_command("$HOME/.cargo/bin/rustup target add x86_64-unknown-linux-musl")
    run_command("$HOME/.cargo/bin/rustup target add aarch64-unknown-linux-gnu")

def install_nodejs():
    """Install latest version of Node.js and pnpm"""
    # Add NodeSource repository
    run_command("curl -fsSL https://deb.nodesource.com/setup_current.x | bash -")
    
    # Install Node.js
    run_command("apt-get install -y nodejs")
    
    # Install pnpm
    run_command("npm install -g pnpm")

def main():
    # Check if apt is available
    check_package_manager()

    # Check if running with root privileges
    if os.geteuid() != 0:
        print("Please run this script with sudo")
        sys.exit(1)

    # Get the actual user (the one running sudo)
    real_user = os.getenv('SUDO_USER')
    if not real_user:
        print("Unable to get actual username")
        sys.exit(1)

    print("Starting development environment setup...")
    
    try:
        # Create /opt/buckyos directory and set owner
        buckyos_dir = "/opt/buckyos"
        if not os.path.exists(buckyos_dir):
            os.makedirs(buckyos_dir)
            print(f"Created directory: {buckyos_dir}")
        
        # Change directory owner to actual user
        run_command(f"chown -R {real_user}:{real_user} {buckyos_dir}")
        print(f"Changed directory owner to: {real_user}")
        
        install_rust_toolchain()
        install_nodejs()
        
        print("\nEnvironment setup completed!")
        print("\nInstalled:")
        print("- Rust toolchain && Build tools")
        print("- OpenSSL dev library")
        print("- docker.io")
        print("- MUSL cross-compilation support")
        print("- AARCH64 cross-compilation support")
        print("- Latest version of Node.js")
        print("- pnpm package manager")
        print(f"- Created directory {buckyos_dir} (owner: {real_user})")
        
    except Exception as e:
        print(f"Error occurred during setup: {str(e)}")
        sys.exit(1)

if __name__ == "__main__":
    main()
