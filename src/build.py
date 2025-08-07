
import subprocess
import platform
import sys
import os

build_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(build_dir, "scripts"))
import build

if __name__ == "__main__":
    build.build_main()