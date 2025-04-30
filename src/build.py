import subprocess
import platform
import sys


subprocess.run(["python3", "scripts/build.py", *sys.argv[1:]], check=True)