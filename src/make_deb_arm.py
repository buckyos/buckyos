import sys
from scripts import make_deb

version = "0.3.0"
if len(sys.argv) > 1:
    version = sys.argv[1]

print(f"make deb with version: {version}")

make_deb.make_deb("arm64", version)