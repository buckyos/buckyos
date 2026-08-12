#!/usr/bin/env -S uv run

import os
import platform
import shlex
import shutil
import subprocess
import sys
from pathlib import Path


DEVKIT_SPEC = "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git"


def _command_names(command: str) -> list[str]:
    if os.name == "nt":
        return [f"{command}.exe", f"{command}.cmd", f"{command}.bat", command]
    return [command]


def _find_command(command: str) -> str | None:
    for name in _command_names(command):
        path = shutil.which(name)
        if path is not None:
            return path

    bin_dir = Path(sys.executable).parent
    for name in _command_names(command):
        candidate = bin_dir / name
        if candidate.exists():
            return str(candidate)

    return None


def _resolve_linux_target(args: list[str]) -> str | None:
    for arg in args:
        if arg == "amd64":
            return "x86_64-unknown-linux-musl"
        if arg == "aarch64":
            return "aarch64-unknown-linux-musl"
        if arg.startswith("--target="):
            target = arg.split("=", 1)[1]
            if "unknown-linux" in target:
                return target
    return None


def _with_extra_clang_arg(current: str | None, extra_arg: str) -> str:
    if current is None or current.strip() == "":
        return extra_arg
    if extra_arg in current:
        return current
    return f"{current} {extra_arg}"


def _target_env_suffix(target: str) -> str:
    return target.replace("-", "_")


def _find_cross_cc(target: str) -> str | None:
    if target == "aarch64-unknown-linux-musl":
        candidates = ["aarch64-linux-musl-gcc"]
    elif target == "x86_64-unknown-linux-musl":
        candidates = ["x86_64-linux-musl-gcc", "musl-gcc"]
    elif target == "aarch64-unknown-linux-gnu":
        candidates = ["aarch64-linux-gnu-gcc"]
    elif target == "x86_64-unknown-linux-gnu":
        candidates = ["x86_64-linux-gnu-gcc"]
    else:
        candidates = []

    for candidate in candidates:
        path = shutil.which(candidate)
        if path:
            return path
    return None


def _find_cross_cxx(target: str) -> str | None:
    if target == "aarch64-unknown-linux-musl":
        candidates = ["aarch64-linux-musl-g++"]
    elif target == "x86_64-unknown-linux-musl":
        candidates = ["x86_64-linux-musl-g++"]
    elif target == "aarch64-unknown-linux-gnu":
        candidates = ["aarch64-linux-gnu-g++"]
    elif target == "x86_64-unknown-linux-gnu":
        candidates = ["x86_64-linux-gnu-g++"]
    else:
        candidates = []

    for candidate in candidates:
        path = shutil.which(candidate)
        if path:
            return path
    return None


def _find_cross_ar(target: str) -> str | None:
    if target == "aarch64-unknown-linux-musl":
        candidates = ["aarch64-linux-musl-ar"]
    elif target == "x86_64-unknown-linux-musl":
        candidates = ["x86_64-linux-musl-ar"]
    elif target == "aarch64-unknown-linux-gnu":
        candidates = ["aarch64-linux-gnu-ar"]
    elif target == "x86_64-unknown-linux-gnu":
        candidates = ["x86_64-linux-gnu-ar"]
    else:
        candidates = []

    for candidate in candidates:
        path = shutil.which(candidate)
        if path:
            return path
    return None


def _compiler_include_args(compiler: str) -> list[str]:
    result = subprocess.run(
        [compiler, "-E", "-Wp,-v", "-x", "c", "-"],
        input="",
        check=False,
        capture_output=True,
        text=True,
    )
    lines = result.stderr.splitlines()
    include_args: list[str] = []
    in_block = False
    for raw_line in lines:
        line = raw_line.rstrip()
        if "#include <...> search starts here:" in line:
            in_block = True
            continue
        if in_block and line.strip() == "End of search list.":
            break
        if in_block:
            path = line.strip()
            if path:
                include_args.extend(["-isystem", path])
    return include_args


def _bindgen_args_for_linux_target(target: str) -> list[str]:
    compiler = _find_cross_cc(target)
    if compiler is None:
        return []

    args = [f"--target={target}"]

    sysroot = subprocess.run(
        [compiler, "-print-sysroot"],
        check=False,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if sysroot:
        args.append(f"--sysroot={sysroot}")

    args.extend(_compiler_include_args(compiler))
    return args


def _toolchain_has_linux_headers(compiler: str) -> bool:
    result = subprocess.run(
        [compiler, "-E", "-x", "c++", "-"],
        input="#include <linux/fs.h>\n",
        check=False,
        capture_output=True,
        text=True,
    )
    return result.returncode == 0


def _build_env(args: list[str]) -> dict[str, str]:
    env = os.environ.copy()
    linux_target = _resolve_linux_target(args)

    if platform.system() == "Darwin" and linux_target:
        target_suffix = _target_env_suffix(linux_target)
        cross_cc = _find_cross_cc(linux_target)
        cross_cxx = _find_cross_cxx(linux_target)
        cross_ar = _find_cross_ar(linux_target)

        if cross_cc:
            env.setdefault(f"CC_{target_suffix}", cross_cc)
        if cross_cxx:
            env.setdefault(f"CXX_{target_suffix}", cross_cxx)
            env[f"CXXFLAGS_{target_suffix}"] = _with_extra_clang_arg(
                env.get(f"CXXFLAGS_{target_suffix}"),
                "-include cstdint",
            )
            env["CXXFLAGS"] = _with_extra_clang_arg(
                env.get("CXXFLAGS"),
                "-include cstdint",
            )
        if cross_ar:
            env.setdefault(f"AR_{target_suffix}", cross_ar)

        bindgen_args = _bindgen_args_for_linux_target(linux_target)
        if bindgen_args:
            bindgen_args_text = shlex.join(bindgen_args)
            env["BINDGEN_EXTRA_CLANG_ARGS"] = _with_extra_clang_arg(
                env.get("BINDGEN_EXTRA_CLANG_ARGS"),
                bindgen_args_text,
            )
            env[f"BINDGEN_EXTRA_CLANG_ARGS_{target_suffix}"] = (
                bindgen_args_text
            )
            print(
                f"* Using cross clang args for bindgen target {linux_target}: {bindgen_args_text}",
                flush=True,
            )

        if cross_cxx and not _toolchain_has_linux_headers(cross_cxx):
            print(
                f"⚠️ {cross_cxx} cannot find Linux kernel headers such as linux/fs.h.",
                flush=True,
            )
            print(
                "   klog/rocksdb will fail to compile until the Linux cross toolchain includes kernel headers.",
                flush=True,
            )
            print(
                "   A complete macOS cross toolchain such as messense/homebrew-macos-cross-toolchains is recommended.",
                flush=True,
            )

    return env


def _run_command(command: str, args: list[str], env: dict[str, str] | None = None) -> int:
    executable = _find_command(command)
    if executable is None:
        print(f"{command} not found in the current uv runtime.")
        print(f"Please re-run this script with `uv run src/buckyos-build.py ...` or install `{DEVKIT_SPEC}`.")
        return 127

    result = subprocess.run([executable] + args, env=env or os.environ.copy())
    return result.returncode


def main() -> int:
    print("!!! buckyos depend on cyfs-gateway, MAKE SURE YOU HAVE BUILD IT FIRST!", flush=True)
    env = _build_env(sys.argv[1:])

    result = _run_command("buckyos-build", sys.argv[1:], env=env)
    if result != 0:
        print(f"buckyos-build failed with return code {result}")
        return result

    result = _run_command("buckyos-update", [], env=env)
    if result != 0:
        print(f"buckyos-update failed with return code {result}")
        return result

    print("buckyos-build and buckyos-update completed successfully")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
