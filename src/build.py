import subprocess
import platform
import sys

if platform.system() == "Windows":
    subprocess.run(["python", "scripts/build.py", *sys.argv[1:]], check=True)
elif platform.system() == "Linux":
    subprocess.run(["python3", "scripts/build.py", "amd64", *sys.argv[1:]], check=True)