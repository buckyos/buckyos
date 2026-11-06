"""`tree` for Windows: no official binary and no conda-forge package exist, and the system's tree.com takes other options.
The options an Agent reaches for: -a, -d, -L <n>, and the same picture and summary line."""
import os
import sys

VERSION = "tree (ext-core) 1.0"


def main(argv):
    show_all = dirs_only = False
    level = None
    roots = []
    args = iter(argv)
    for arg in args:
        if arg == "--version":
            print(VERSION)
            return 0
        if arg == "-a":
            show_all = True
        elif arg == "-d":
            dirs_only = True
        elif arg == "-L":
            level = int(next(args, "0"))
        elif arg.startswith("-"):
            print(f"tree: unsupported option {arg}", file=sys.stderr)
            return 2
        else:
            roots.append(arg)
    counts = [0, 0]

    def walk(path, prefix, depth):
        if level is not None and depth >= level:
            return
        try:
            names = sorted(os.listdir(path), key=str.lower)
        except OSError as error:
            print(f"{prefix}[{error.strerror}]")
            return
        if not show_all:
            names = [n for n in names if not n.startswith(".")]
        if dirs_only:
            names = [n for n in names if os.path.isdir(os.path.join(path, n))]
        for index, name in enumerate(names):
            last = index == len(names) - 1
            print(f"{prefix}{'└── ' if last else '├── '}{name}")
            full = os.path.join(path, name)
            if os.path.isdir(full):
                counts[0] += 1
                walk(full, prefix + ("    " if last else "│   "), depth + 1)
            else:
                counts[1] += 1

    for root in roots or ["."]:
        print(root)
        walk(root, "", 0)
    print(f"\n{counts[0]} directories" + ("" if dirs_only else f", {counts[1]} files"))
    return 0


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    sys.exit(main(sys.argv[1:]))
