
import os
import shutil
import subprocess
import sys
import platform

current_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.append(os.path.join(current_dir, "scripts"))
import install

def main():
    if len(sys.argv) < 2:
        print("Usage: python switch.py <config_group_name>")
        sys.exit(1)
    config_group_name = sys.argv[1]
    install.copy_configs(config_group_name)

if __name__ == "__main__":
    main()
