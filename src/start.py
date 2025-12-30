
import os
import shutil
import subprocess
import sys
import platform
from pathlib import Path
import buckyos_devkit
from make_config import make_config_by_group_name

build_dir = os.path.dirname(os.path.abspath(__file__))

# after run build.py ,use this script to restart the dev buckyos system
# 1) killall process
# 2) update files to /opt/buckyos (--all to update all files to /opt/buckyos)
# 3) start the system (run /opt/buckyos/bin/node_daemon/node_daemon)

def kill_all_processes():
    """Kill all related BuckyOS processes"""
    print("Stopping all BuckyOS processes...")
    
    # Import and execute killall.py functions directly
    try:
        import stop
        # Execute the main logic of killall.py
        stop.kill_all()
        print("All processes stopped")
    except ImportError as e:
        print(f"Failed to import killall module: {e}")
        sys.exit(1)
    except Exception as e:
        print(f"Warning: Some processes may not have been stopped: {e}")
        # Continue execution even if some processes fail to stop

def update_files(install_all=False,config_group_name=None):
    """Update files to installation directory"""
    print("Updating files...")
    
    # Import and call install.py functions directly

    try:
        install_cmd = ["buckyos-update", "--app=buckyos"]
        if install_all:
            install_cmd.append("--all")
        subprocess.run(install_cmd, env=os.environ.copy())

        if config_group_name:
           target_root : Path = Path("/opt/buckyos")
           make_config_by_group_name(config_group_name, target_root, None, None, None)
        print("Files updated successfully")
    except ImportError as e:
        print(f"Failed to import install module: {e}")
        sys.exit(1)
    except Exception as e:
        print(f"Failed to update files: {e}")
        sys.exit(1)
    finally:
        # Remove the added path
        sys.path.pop(0)


    

def start_system():
    """Start BuckyOS system"""
    print("Starting BuckyOS system...")
    
    # Get BUCKYOS_ROOT environment variable or use default installation directory
    buckyos_root = os.environ.get('BUCKYOS_ROOT')
    if not buckyos_root:
        # Use default installation directory if BUCKYOS_ROOT is not set
        if platform.system() == "Windows":
            buckyos_root = os.path.join(os.path.expandvars("%AppData%"), "buckyos")
        else:
            buckyos_root = "/opt/buckyos"
        print(f"BUCKYOS_ROOT not set, using default: {buckyos_root}")
    else:
        print(f"Using BUCKYOS_ROOT: {buckyos_root}")
    
    # Start node_daemon
    node_daemon_path = os.path.join(buckyos_root, "bin", "node-daemon", "node_daemon")
    
    if platform.system() == "Windows":
        node_daemon_path += ".exe"
    
    if not os.path.exists(node_daemon_path):
        print(f"Error: Cannot find node_daemon executable: {node_daemon_path}")
        print(f"Please check if the installation directory is correct: {buckyos_root}")
        sys.exit(1)
    
    try:
        # Start node_daemon in background with BUCKYOS_ROOT environment
        env = os.environ.copy()
        env['BUCKYOS_ROOT'] = buckyos_root
        
        if platform.system() == "Windows":
            subprocess.Popen([node_daemon_path,"--enable_active"], 
                           env=env,
                           creationflags=subprocess.CREATE_NEW_CONSOLE)
        else:
            subprocess.Popen([node_daemon_path,"--enable_active"], 
                           env=env,
                           stdout=subprocess.DEVNULL, 
                           stderr=subprocess.DEVNULL)
        
        print(f"BuckyOS system started: {node_daemon_path}")
        print("System is running in background...")
        
    except Exception as e:
        print(f"Failed to start system: {e}")
        sys.exit(1)

def main():
    """Main function"""
    print("=== BuckyOS Development Environment Startup Script ===")
    
    # Parse command line arguments
    config_group_name = None
    install_all = "--all" in sys.argv or "--reinstall" in sys.argv
    need_update = "--skip-update" not in sys.argv
    if install_all:
        config_group_name = "dev"
    if "--reinstall" in sys.argv:
        config_group_name = None
        group_name_index = sys.argv.index("--reinstall") + 1
        if group_name_index < len(sys.argv):
            config_group_name = sys.argv[group_name_index]
    
    # Step 1: Kill all processes
    kill_all_processes()
    
    if install_all or need_update:
        # Step 2: Update files
        update_files(install_all,config_group_name)
    
    # Step 3: Start system
    start_system()
    
    print("=== BuckyOS Startup Complete ===")

if __name__ == "__main__":
    main()
