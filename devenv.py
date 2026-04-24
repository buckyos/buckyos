#!/usr/bin/env python3
from __future__ import annotations

import argparse
from dataclasses import dataclass, field
import hashlib
import os
from pathlib import Path
import platform
import shlex
import shutil
import subprocess
import sys
import tempfile
import tarfile
from typing import Callable, Sequence
from urllib.request import urlopen

if os.name != "nt":
    import grp
    import pwd


class BootstrapError(RuntimeError):
    pass


LINUX_CORE_PACKAGES = {
    "apt-get": [
        "build-essential",
        "curl",
        "wget",
        "git",
        "pkg-config",
        "libssl-dev",
        "clang",
        "libclang-dev",
        "llvm-dev",
    ],
    "dnf": ["gcc", "gcc-c++", "make", "curl", "wget", "git", "pkgconf-pkg-config", "openssl-devel"],
    "yum": ["gcc", "gcc-c++", "make", "curl", "wget", "git", "pkgconfig", "openssl-devel"],
    "pacman": ["base-devel", "curl", "wget", "git", "pkgconf", "openssl"],
    "zypper": ["gcc", "gcc-c++", "make", "curl", "wget", "git", "pkg-config", "libopenssl-devel"],
}

LINUX_PYTHON_CHOICES = {
    "apt-get": [["python3", "python3-pip", "python3-venv"]],
    "dnf": [["python3", "python3-pip"]],
    "yum": [["python3", "python3-pip"]],
    "pacman": [["python", "python-pip"]],
    "zypper": [["python312", "python312-pip"], ["python311", "python311-pip"], ["python3", "python3-pip"]],
}

NODEJS_LINUX_LTS_MAJOR = 24
NODEJS_DIST_BASE_URL = "https://nodejs.org/dist"

LINUX_RUSTUP_CHOICES = {
    "apt-get": [["rustup"]],
    "dnf": [["rustup"]],
    "yum": [["rustup"]],
    "pacman": [["rustup"]],
    "zypper": [["rustup"]],
}

LINUX_DOCKER_CHOICES = {
    "apt-get": [["docker.io"], ["moby-engine"]],
    "dnf": [["moby-engine"], ["docker"], ["docker-ce"]],
    "yum": [["docker"], ["docker-ce"], ["moby-engine"]],
    "pacman": [["docker"]],
    "zypper": [["docker"], ["docker-ce"], ["moby-engine"]],
}

LINUX_PNPM_CHOICES = {
    "apt-get": [["pnpm"]],
    "dnf": [["pnpm"]],
    "yum": [["pnpm"]],
    "pacman": [["pnpm"]],
    "zypper": [["pnpm"]],
}

LINUX_UV_CHOICES = {
    "apt-get": [["uv"]],
    "dnf": [["uv"]],
    "yum": [["uv"]],
    "pacman": [["uv"]],
    "zypper": [["uv"]],
}

LINUX_DENO_CHOICES = {
    "apt-get": [["deno"]],
    "dnf": [["deno"]],
    "yum": [["deno"]],
    "pacman": [["deno"]],
    "zypper": [["deno"]],
}

LINUX_TMUX_CHOICES = {
    "apt-get": [["tmux"]],
    "dnf": [["tmux"]],
    "yum": [["tmux"]],
    "pacman": [["tmux"]],
    "zypper": [["tmux"]],
}

LINUX_CROSS_PACKAGE_CHOICES = {
    "apt-get": [["musl-tools"], ["gcc-aarch64-linux-gnu"]],
    "dnf": [["musl-gcc"], ["gcc-aarch64-linux-gnu"]],
    "yum": [["musl-gcc"], ["gcc-aarch64-linux-gnu"]],
    "pacman": [["musl"], ["musl-aarch64"]],
    "zypper": [["musl"], ["gcc-aarch64-linux-gnu"], ["cross-aarch64-gcc13", "cross-aarch64-binutils"]],
}

MUSL_CROSS_INSTALL_ROOT = Path("/opt/musl-cross")
MUSL_CROSS_BASE_URL = "https://musl.cc"
MUSL_CROSS_TOOLCHAINS = [
    ("x86_64-linux-musl", "x86_64-linux-musl-cross"),
    ("aarch64-linux-musl", "aarch64-linux-musl-cross"),
]

LINUX_BINDGEN_PACKAGE_CHOICES = {
    "apt-get": [["clang", "libclang-dev", "llvm-dev"]],
    "dnf": [
        ["clang", "clang-devel", "llvm-devel"],
        ["clang", "libclang", "llvm-devel"],
        ["clang", "libclang-devel", "llvm-devel"],
    ],
    "yum": [
        ["clang", "clang-devel", "llvm-devel"],
        ["clang", "libclang", "llvm-devel"],
        ["clang", "libclang-devel", "llvm-devel"],
    ],
    "pacman": [["clang", "llvm"]],
    "zypper": [
        ["clang", "clang-devel", "llvm-devel"],
        ["clang", "libclang-devel", "llvm-devel"],
        ["clang", "llvm-devel"],
    ],
}

BREW_FORMULAE = ["git", "wget", "pkgconf", "openssl@3", "python@3.12", "node", "pnpm", "rustup", "uv", "deno", "tmux"]
BREW_CASKS = ["docker"]

WINGET_PACKAGE_CHOICES = {
    "git": [["Git.Git"]],
    "python": [["Python.Python.3.12"], ["Python.Python.3.11"], ["Python.Python.3"]],
    "node": [["OpenJS.NodeJS.LTS"], ["OpenJS.NodeJS"]],
    "pnpm": [["pnpm.pnpm"]],
    "rustup": [["Rustlang.Rustup"]],
    "uv": [["astral-sh.uv"]],
    "deno": [["DenoLand.Deno"]],
    "docker": [["Docker.DockerDesktop"]],
    "msvc": [["Microsoft.VisualStudio.2022.BuildTools"]],
}

WINGET_MSVS_OVERRIDE = "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"


@dataclass
class Bootstrapper:
    args: argparse.Namespace
    system: str = field(init=False)
    package_manager: str = field(init=False)
    notes: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)

    def __post_init__(self) -> None:
        self.system = platform.system()
        self.package_manager = self.detect_package_manager()

    def detect_package_manager(self) -> str:
        if self.system == "Windows":
            if shutil.which("winget"):
                return "winget"
            raise BootstrapError("Windows requires winget (App Installer)")

        if self.system == "Darwin":
            if shutil.which("brew"):
                return "brew"
            raise BootstrapError("macOS requires Homebrew to be installed first")

        if self.system == "Linux":
            for manager in ("apt-get", "dnf", "yum", "pacman", "zypper"):
                if shutil.which(manager):
                    return manager
            raise BootstrapError("No supported default package manager detected for this Linux distribution")

        raise BootstrapError(f"Unsupported operating system: {self.system}")

    def run(self, command: Sequence[str], check: bool = True, capture_output: bool = False) -> subprocess.CompletedProcess[str]:
        display = self.format_command(command)
        print(f"> {display}")
        if self.args.dry_run:
            return subprocess.CompletedProcess(command, 0, "", "")

        result = subprocess.run(
            list(command),
            check=False,
            text=True,
            capture_output=capture_output,
        )
        if check and result.returncode != 0:
            stderr = (result.stderr or "").strip()
            stdout = (result.stdout or "").strip()
            detail = stderr or stdout
            if detail:
                raise BootstrapError(f"Command failed: {display}\n{detail}")
            raise BootstrapError(f"Command failed: {display}")
        return result

    def probe(self, command: Sequence[str]) -> bool:
        result = subprocess.run(list(command), check=False, text=True, capture_output=True)
        return result.returncode == 0

    def capture_text(self, command: Sequence[str]) -> str:
        result = subprocess.run(list(command), check=False, text=True, capture_output=True)
        if result.returncode != 0:
            return ""
        return (result.stdout or "").strip()

    def format_command(self, command: Sequence[str]) -> str:
        if os.name == "nt":
            return subprocess.list2cmdline(list(command))
        return shlex.join(list(command))

    def print_command(self, command: Sequence[str]) -> None:
        print(f"> {self.format_command(command)}")

    def require_privilege(self, command: Sequence[str]) -> list[str]:
        if self.package_manager in {"apt-get", "dnf", "yum", "pacman", "zypper"}:
            if hasattr(os, "geteuid") and os.geteuid() == 0:
                return list(command)
            if shutil.which("sudo"):
                return ["sudo", *command]
            raise BootstrapError("Please use root privileges or ensure sudo is available")
        return list(command)

    def require_unix_privilege(self, command: Sequence[str]) -> list[str]:
        if self.system == "Windows":
            return list(command)
        if hasattr(os, "geteuid") and os.geteuid() == 0:
            return list(command)
        if shutil.which("sudo"):
            return ["sudo", *command]
        raise BootstrapError("Please use administrator privileges or ensure sudo is available")

    def run_as_invoking_user(self, command: Sequence[str]) -> list[str]:
        if self.system == "Windows":
            return list(command)

        if hasattr(os, "geteuid") and os.geteuid() == 0:
            sudo_user = os.environ.get("SUDO_USER")
            if sudo_user:
                if shutil.which("sudo"):
                    return ["sudo", "-H", "-u", sudo_user, *command]
                raise BootstrapError("sudo is required to run user-level installers as the invoking user")

        return list(command)

    def invoking_user_home(self) -> Path:
        if self.system != "Windows" and hasattr(os, "geteuid") and os.geteuid() == 0:
            sudo_user = os.environ.get("SUDO_USER")
            if sudo_user:
                try:
                    return Path(pwd.getpwnam(sudo_user).pw_dir)
                except KeyError:
                    pass

        if self.system == "Windows":
            return Path(os.environ.get("USERPROFILE", str(Path.home())))

        return Path.home()

    def package_installed(self, package: str, kind: str = "package") -> bool:
        if self.package_manager == "apt-get":
            return self.probe(["dpkg", "-s", package])
        if self.package_manager in {"dnf", "yum", "zypper"}:
            return self.probe(["rpm", "-q", package])
        if self.package_manager == "pacman":
            return self.probe(["pacman", "-Qi", package])
        if self.package_manager == "brew":
            if kind == "cask":
                return self.probe(["brew", "list", "--cask", package])
            output = self.capture_text(["brew", "list", "--versions", package])
            return bool(output)
        if self.package_manager == "winget":
            result = subprocess.run(
                ["winget", "list", "--id", package, "--exact", "--accept-source-agreements"],
                check=False,
                text=True,
                capture_output=True,
            )
            return result.returncode == 0 and package.lower() in (result.stdout or "").lower()
        return False

    def package_available(self, package: str, kind: str = "package") -> bool:
        if self.package_manager == "apt-get":
            return self.probe(["apt-cache", "show", package])
        if self.package_manager == "dnf":
            return self.probe(["dnf", "info", package])
        if self.package_manager == "yum":
            return self.probe(["yum", "info", package])
        if self.package_manager == "pacman":
            return self.probe(["pacman", "-Si", package])
        if self.package_manager == "zypper":
            return self.probe(["zypper", "info", package])
        if self.package_manager == "brew":
            if kind == "cask":
                return self.probe(["brew", "info", "--cask", package])
            return self.probe(["brew", "info", package])
        if self.package_manager == "winget":
            return self.probe(["winget", "show", "--id", package, "--exact", "--accept-source-agreements"])
        return False

    def resolve_package_set(self, choices: list[list[str]], kind: str = "package") -> list[str] | None:
        for packages in choices:
            if all(self.package_installed(package, kind=kind) for package in packages):
                return packages

        for packages in choices:
            if all(self.package_available(package, kind=kind) for package in packages):
                return packages

        return None

    def update_package_index(self) -> None:
        if self.package_manager == "apt-get":
            self.run(self.require_privilege(["apt-get", "update"]))
            return
        if self.package_manager == "dnf":
            self.run(self.require_privilege(["dnf", "makecache"]))
            return
        if self.package_manager == "yum":
            self.run(self.require_privilege(["yum", "makecache"]))
            return
        if self.package_manager == "pacman":
            self.run(self.require_privilege(["pacman", "-Sy", "--noconfirm"]))
            return
        if self.package_manager == "zypper":
            self.run(self.require_privilege(["zypper", "--gpg-auto-import-keys", "--non-interactive", "refresh"]))
            return
        if self.package_manager == "brew":
            self.run(["brew", "update"])
            return
        if self.package_manager == "winget":
            self.run(["winget", "source", "update"])
            return

    def install_packages(self, packages: list[str], kind: str = "package") -> None:
        if not packages:
            return

        if self.package_manager == "apt-get":
            self.run(self.require_privilege(["apt-get", "install", "-y", *packages]))
            return
        if self.package_manager == "dnf":
            self.run(self.require_privilege(["dnf", "install", "-y", *packages]))
            return
        if self.package_manager == "yum":
            self.run(self.require_privilege(["yum", "install", "-y", *packages]))
            return
        if self.package_manager == "pacman":
            self.run(self.require_privilege(["pacman", "-S", "--noconfirm", "--needed", *packages]))
            return
        if self.package_manager == "zypper":
            self.run(self.require_privilege(["zypper", "--non-interactive", "install", "--no-recommends", *packages]))
            return
        if self.package_manager == "brew":
            if kind == "cask":
                self.run(["brew", "install", "--cask", *packages])
            else:
                self.run(["brew", "install", *packages])
            return
        raise BootstrapError("install_packages is not applicable for current package manager")

    def install_winget_package(self, package_id: str, override: str | None = None) -> None:
        command = [
            "winget",
            "install",
            "--id",
            package_id,
            "--exact",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--silent",
        ]
        if override:
            command.extend(["--override", override])
        self.run(command)

    def install_first_resolved_set(
        self,
        description: str,
        choices: list[list[str]],
        kind: str = "package",
        optional: bool = False,
    ) -> list[str] | None:
        packages = self.resolve_package_set(choices, kind=kind)
        if not packages:
            message = f"No available {description} package found"
            if optional:
                self.warnings.append(message)
                return None
            raise BootstrapError(message)

        missing = [package for package in packages if not self.package_installed(package, kind=kind)]
        if missing:
            self.install_packages(missing, kind=kind)
        return packages

    def ensure_unix_script_tool(self, description: str, script_url: str, locator: Callable[[], str | None]) -> None:
        if locator():
            return

        if hasattr(os, "geteuid") and os.geteuid() == 0 and not os.environ.get("SUDO_USER"):
            self.warnings.append(
                f"{description} will be installed into root's home directory because the script is running as root"
            )

        if shutil.which("curl"):
            fetch_command = f"curl -LsSf {shlex.quote(script_url)} | sh"
        elif shutil.which("wget"):
            fetch_command = f"wget -qO- {shlex.quote(script_url)} | sh"
        else:
            raise BootstrapError(f"{description} installer requires curl or wget")

        self.run(self.run_as_invoking_user(["sh", "-c", fetch_command]))

        tool_path = locator()
        if tool_path and not shutil.which(Path(tool_path).name):
            self.notes.append(f"{description} was installed at {tool_path}; reopen the terminal if it is not yet in PATH")

    def find_uv(self) -> str | None:
        candidates = [
            Path(self.invoking_user_home()) / ".local" / "bin" / "uv",
            Path(self.invoking_user_home()) / ".cargo" / "bin" / "uv",
            Path("/opt/homebrew/bin/uv"),
            Path("/usr/local/bin/uv"),
            Path("/home/linuxbrew/.linuxbrew/bin/uv"),
        ]
        return self.find_binary("uv", candidates)

    def find_deno(self) -> str | None:
        candidates = [
            Path(self.invoking_user_home()) / ".deno" / "bin" / "deno",
            Path("/opt/homebrew/bin/deno"),
            Path("/usr/local/bin/deno"),
            Path("/home/linuxbrew/.linuxbrew/bin/deno"),
        ]
        return self.find_binary("deno", candidates)

    def find_corepack(self) -> str | None:
        candidates = [
            Path("/usr/bin/corepack"),
            Path("/usr/local/bin/corepack"),
            Path("/opt/homebrew/bin/corepack"),
            Path("/home/linuxbrew/.linuxbrew/bin/corepack"),
        ]
        return self.find_binary("corepack", candidates)

    def find_binary(self, binary_name: str, candidates: Sequence[Path]) -> str | None:
        if path := shutil.which(binary_name):
            return path

        for candidate in candidates:
            if candidate.exists():
                return str(candidate)
        return None

    def find_libclang(self) -> str | None:
        env_path = os.environ.get("LIBCLANG_PATH")
        if env_path:
            env_candidate = Path(env_path)
            search_roots = [env_candidate] if env_candidate.is_dir() else [env_candidate.parent]
        else:
            search_roots = []

        search_roots.extend(
            [
                Path("/usr/lib"),
                Path("/usr/local/lib"),
                Path("/usr/lib64"),
                Path("/usr/local/lib64"),
                Path("/opt/homebrew/opt/llvm/lib"),
                Path("/usr/local/opt/llvm/lib"),
            ]
        )
        search_roots.extend(sorted(Path("/usr/lib").glob("llvm-*/lib")))
        search_roots.extend(sorted(Path("/usr/lib64").glob("llvm-*/lib")))

        seen: set[Path] = set()
        for root in search_roots:
            if root in seen or not root.exists():
                continue
            seen.add(root)
            for pattern in ("libclang.so", "libclang.so.*", "libclang.dylib", "libclang.dll"):
                matches = sorted(root.glob(pattern))
                if matches:
                    return str(matches[0])
        return None

    def ensure_uv(self) -> None:
        if self.find_uv():
            return

        if self.system == "Linux":
            packages = self.resolve_package_set(LINUX_UV_CHOICES[self.package_manager])
            if packages:
                missing = [package for package in packages if not self.package_installed(package)]
                if missing:
                    self.install_packages(missing)
            else:
                self.ensure_unix_script_tool("uv", "https://astral.sh/uv/install.sh", self.find_uv)
            return

        if self.system == "Windows":
            package_ids = self.resolve_package_set(WINGET_PACKAGE_CHOICES["uv"], kind="winget")
            if not package_ids:
                raise BootstrapError("winget could not find the package for uv")
            package_id = package_ids[0]
            if not self.package_installed(package_id, kind="winget"):
                self.install_winget_package(package_id)

    def ensure_deno(self) -> None:
        if self.find_deno():
            return

        if self.system == "Linux":
            packages = self.resolve_package_set(LINUX_DENO_CHOICES[self.package_manager])
            if packages:
                missing = [package for package in packages if not self.package_installed(package)]
                if missing:
                    self.install_packages(missing)
            else:
                self.ensure_unix_script_tool("Deno", "https://deno.land/install.sh", self.find_deno)
            return

        if self.system == "Windows":
            package_ids = self.resolve_package_set(WINGET_PACKAGE_CHOICES["deno"], kind="winget")
            if not package_ids:
                raise BootstrapError("winget could not find the package for Deno")
            package_id = package_ids[0]
            if not self.package_installed(package_id, kind="winget"):
                self.install_winget_package(package_id)

    def ensure_linux_rustup(self) -> None:
        if self.find_rustup():
            return

        packages = self.resolve_package_set(LINUX_RUSTUP_CHOICES[self.package_manager])
        if packages:
            missing = [package for package in packages if not self.package_installed(package)]
            if missing:
                self.install_packages(missing)
            return

        if hasattr(os, "geteuid") and os.geteuid() == 0 and not os.environ.get("SUDO_USER"):
            self.warnings.append(
                "rustup will be installed into root's home directory because the script is running as root"
            )

        if shutil.which("curl"):
            fetch_command = "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
        elif shutil.which("wget"):
            fetch_command = "wget -qO- https://sh.rustup.rs | sh -s -- -y"
        else:
            raise BootstrapError("rustup installer requires curl or wget")

        self.run(self.run_as_invoking_user(["sh", "-c", fetch_command]))

        rustup_path = self.find_rustup()
        if rustup_path and not shutil.which(Path(rustup_path).name):
            self.notes.append(f"rustup was installed at {rustup_path}; reopen the terminal if it is not yet in PATH")

    def ensure_tmux(self) -> None:
        if shutil.which("tmux"):
            return

        if self.system == "Windows":
            self.warnings.append("tmux is needed by OpenDAN Runtime; use WSL2 if you need tmux;Otherwise, you will not be able to debug and run Jarvis under Windows. You can only officially run Jarvis using Docker.")
            return

        if self.system == "Linux":
            self.install_first_resolved_set("tmux", LINUX_TMUX_CHOICES[self.package_manager])

    def installed_node_version(self) -> str | None:
        node_path = shutil.which("node")
        if not node_path:
            return None
        version = self.capture_text([node_path, "--version"])
        return version or None

    def resolve_linux_node_arch(self) -> str:
        machine = platform.machine().lower()
        mapping = {
            "x86_64": "x64",
            "amd64": "x64",
            "aarch64": "arm64",
            "arm64": "arm64",
            "armv7l": "armv7l",
        }
        arch = mapping.get(machine)
        if not arch:
            raise BootstrapError(f"Unsupported Linux architecture for Node.js binary install: {machine}")
        return arch

    def fetch_text(self, url: str) -> str:
        with urlopen(url) as response:
            return response.read().decode("utf-8")

    def download_file(self, url: str, target: Path) -> None:
        with urlopen(url) as response, target.open("wb") as output:
            shutil.copyfileobj(response, output)

    def ensure_linux_node(self) -> None:
        installed_version = self.installed_node_version()
        if installed_version is not None:
            self.notes.append(f"Using existing Node.js installation: {installed_version}")
            return

        target_major = NODEJS_LINUX_LTS_MAJOR
        arch = self.resolve_linux_node_arch()
        base_url = f"{NODEJS_DIST_BASE_URL}/latest-v{target_major}.x"
        shasums_url = f"{base_url}/SHASUMS256.txt"

        if self.args.dry_run:
            self.print_command(["curl", "-fsSL", shasums_url])
            self.print_command(["curl", "-fsSLO", f"{base_url}/node-<version>-linux-{arch}.tar.xz"])
            self.print_command(["sudo", "tar", "-xJf", "node-<version>-linux-<arch>.tar.xz", "-C", "/usr/local/lib/nodejs"])
            self.print_command(["sudo", "ln", "-sfn", "/usr/local/lib/nodejs/node-<version>-linux-<arch>/bin/node", "/usr/local/bin/node"])
            self.print_command(["sudo", "ln", "-sfn", "/usr/local/lib/nodejs/node-<version>-linux-<arch>/bin/npm", "/usr/local/bin/npm"])
            self.print_command(["sudo", "ln", "-sfn", "/usr/local/lib/nodejs/node-<version>-linux-<arch>/bin/npx", "/usr/local/bin/npx"])
            self.print_command(["sudo", "ln", "-sfn", "/usr/local/lib/nodejs/node-<version>-linux-<arch>/bin/corepack", "/usr/local/bin/corepack"])
            self.print_command(["sudo", "corepack", "enable", "pnpm"])
            return

        try:
            shasums_text = self.fetch_text(shasums_url)
        except Exception as error:
            raise BootstrapError(f"Failed to query Node.js release metadata from {shasums_url}: {error}") from error

        archive_name = None
        expected_sha256 = None
        archive_suffix = f"linux-{arch}.tar.xz"
        for line in shasums_text.splitlines():
            parts = line.strip().split()
            if len(parts) != 2:
                continue
            sha256, filename = parts
            if filename.endswith(archive_suffix):
                archive_name = filename
                expected_sha256 = sha256
                break

        if not archive_name or not expected_sha256:
            raise BootstrapError(f"Could not find a Linux Node.js archive for architecture {arch} at {shasums_url}")

        with tempfile.TemporaryDirectory(prefix="buckyos-node-") as temp_dir:
            temp_path = Path(temp_dir)
            archive_path = temp_path / archive_name
            extract_root = temp_path / "extract"
            extract_root.mkdir(parents=True, exist_ok=True)

            archive_url = f"{base_url}/{archive_name}"
            try:
                self.download_file(archive_url, archive_path)
            except Exception as error:
                raise BootstrapError(f"Failed to download Node.js archive from {archive_url}: {error}") from error

            file_sha256 = hashlib.sha256(archive_path.read_bytes()).hexdigest()
            if file_sha256 != expected_sha256:
                raise BootstrapError(
                    f"Downloaded Node.js archive checksum mismatch for {archive_name}: expected {expected_sha256}, got {file_sha256}"
                )

            try:
                with tarfile.open(archive_path, "r:xz") as archive:
                    archive.extractall(extract_root)
            except Exception as error:
                raise BootstrapError(f"Failed to extract Node.js archive {archive_name}: {error}") from error

            extracted_dirs = [path for path in extract_root.iterdir() if path.is_dir()]
            if len(extracted_dirs) != 1:
                raise BootstrapError(f"Unexpected Node.js archive layout in {archive_name}")

            extracted_dir = extracted_dirs[0]
            install_root = Path("/usr/local/lib/nodejs")
            install_dir = install_root / extracted_dir.name

            self.run(self.require_unix_privilege(["mkdir", "-p", str(install_root)]))
            if not install_dir.exists():
                self.run(self.require_unix_privilege(["mv", str(extracted_dir), str(install_dir)]))

            for binary in ("node", "npm", "npx", "corepack"):
                self.run(
                    self.require_unix_privilege(
                        ["ln", "-sfn", str(install_dir / "bin" / binary), f"/usr/local/bin/{binary}"]
                    )
                )

            self.notes.append(f"Installed Node.js from official binaries at {install_dir}")

    def try_enable_pnpm_via_corepack(self) -> bool:
        corepack = self.find_corepack()
        if not corepack:
            return False

        command = [corepack, "enable", "pnpm"]
        if self.system == "Linux":
            command = self.require_unix_privilege(command)

        result = self.run(command, check=False, capture_output=True)
        if result.returncode == 0:
            if not shutil.which("pnpm"):
                self.notes.append("pnpm was enabled via corepack; reopen the terminal if it is not yet in PATH")
            return True

        detail = ((result.stderr or "").strip() or (result.stdout or "").strip())
        if detail:
            self.warnings.append(f"Failed to enable pnpm via corepack: {detail}")
        else:
            self.warnings.append("Failed to enable pnpm via corepack")
        return False

    def install_linux_environment(self) -> None:
        self.update_package_index()
        self.install_packages(LINUX_CORE_PACKAGES[self.package_manager])
        if self.package_manager != "apt-get":
            self.install_first_resolved_set(
                "clang/libclang build dependencies",
                LINUX_BINDGEN_PACKAGE_CHOICES[self.package_manager],
                optional=True,
            )
        self.install_first_resolved_set("Python 3", LINUX_PYTHON_CHOICES[self.package_manager])
        self.ensure_linux_node()
        self.ensure_linux_rustup()
        self.ensure_uv()
        self.ensure_deno()
        self.ensure_tmux()

        if not self.args.skip_docker:
            self.install_first_resolved_set("Docker", LINUX_DOCKER_CHOICES[self.package_manager], optional=True)

        if not shutil.which("pnpm"):
            if self.try_enable_pnpm_via_corepack():
                pass
            else:
                package_set = self.install_first_resolved_set(
                    "pnpm",
                    LINUX_PNPM_CHOICES[self.package_manager],
                    optional=True,
                )
                if package_set is not None:
                    pass
                elif shutil.which("npm"):
                    self.run(self.require_privilege(["npm", "install", "-g", "pnpm"]))
                else:
                    self.warnings.append("Node.js is installed but neither corepack, pnpm package, nor npm was available; please install pnpm manually")

        self.ensure_rust_toolchain()

        if not self.args.skip_cross_tools:
            for choices in LINUX_CROSS_PACKAGE_CHOICES[self.package_manager]:
                self.install_first_resolved_set("cross-compilation dependencies", [choices], optional=True)
            self.ensure_linux_full_musl_toolchain()
            self.check_linux_static_tooling()
        self.check_linux_bindgen_tooling()

        if not self.args.skip_buckyos_dir:
            self.ensure_buckyos_directory()

        if not self.args.skip_docker:
            self.notes.append("To use Docker without sudo, add the current user to the docker group and log in again")

    def install_macos_environment(self) -> None:
        self.update_package_index()
        self.ensure_macos_build_tools()

        missing_formulae = [package for package in BREW_FORMULAE if not self.package_installed(package)]
        if missing_formulae:
            self.install_packages(missing_formulae)

        if not self.args.skip_docker:
            missing_casks = [package for package in BREW_CASKS if not self.package_installed(package, kind="cask")]
            if missing_casks:
                self.install_packages(missing_casks, kind="cask")
            self.notes.append("Please manually start Docker Desktop at least once after installation")

        self.ensure_rust_toolchain()
        self.ensure_uv()
        self.ensure_deno()
        self.ensure_tmux()

        if not self.args.skip_cross_tools:
            self.ensure_macos_musl_cross()

        if not self.args.skip_buckyos_dir:
            self.ensure_buckyos_directory()

        rustup_prefix = self.capture_text(["brew", "--prefix", "rustup"])
        if rustup_prefix:
            self.notes.append(f"If rustup is not found in terminal, add {rustup_prefix}/bin to PATH")

    def ensure_macos_musl_cross(self) -> None:
        """Install FiloSottile/musl-cross so Linux musl cross-compilation works on macOS.

        The formula is keg-only; build_aios auto-detects binaries under
        /opt/homebrew/opt/musl-cross/bin (or /usr/local/opt/musl-cross/bin on Intel),
        so no PATH changes are required.
        """
        if self.package_installed("musl-cross"):
            return
        self.install_packages(["FiloSottile/musl-cross/musl-cross"])
        self.notes.append(
            "Installed FiloSottile/musl-cross (keg-only); build_aios discovers it under "
            "/opt/homebrew/opt/musl-cross/bin without PATH changes."
        )

    def install_windows_environment(self) -> None:
        self.update_package_index()

        for feature in ("git", "python", "node", "pnpm", "rustup", "uv", "deno"):
            package_ids = self.resolve_package_set(WINGET_PACKAGE_CHOICES[feature], kind="winget")
            if not package_ids:
                raise BootstrapError(f"winget could not find the package for {feature}")
            package_id = package_ids[0]
            if not self.package_installed(package_id, kind="winget"):
                self.install_winget_package(package_id)

        self.ensure_tmux()

        if not self.args.skip_docker:
            package_ids = self.resolve_package_set(WINGET_PACKAGE_CHOICES["docker"], kind="winget")
            if package_ids:
                package_id = package_ids[0]
                if not self.package_installed(package_id, kind="winget"):
                    self.install_winget_package(package_id)
                self.notes.append("Docker Desktop may require enabling virtualization or a system restart on first launch")
            else:
                self.warnings.append("winget could not find Docker Desktop, skipped")

        if not self.args.skip_msvc:
            package_ids = self.resolve_package_set(WINGET_PACKAGE_CHOICES["msvc"], kind="winget")
            if package_ids:
                package_id = package_ids[0]
                if not self.package_installed(package_id, kind="winget"):
                    self.install_winget_package(package_id, override=WINGET_MSVS_OVERRIDE)
                self.notes.append("It is recommended to reopen the terminal after MSVC Build Tools installation")
            else:
                self.warnings.append("winget could not find Visual Studio Build Tools; native Windows Rust build may fail")

        self.ensure_rust_toolchain()
        self.notes.append("For Linux target cross-compilation on Windows, consider using WSL2")

    def musl_toolchain_complete(self, prefix: str) -> bool:
        for tool in ("gcc", "g++", "ar", "ranlib"):
            name = f"{prefix}-{tool}"
            if shutil.which(name):
                continue
            if (MUSL_CROSS_INSTALL_ROOT / f"{prefix}-cross" / "bin" / name).exists():
                continue
            return False
        return True

    def ensure_linux_full_musl_toolchain(self) -> None:
        for prefix, archive_name in MUSL_CROSS_TOOLCHAINS:
            if self.musl_toolchain_complete(prefix):
                continue
            try:
                self.install_musl_cross_archive(archive_name)
            except BootstrapError as error:
                self.warnings.append(str(error))

    def install_musl_cross_archive(self, archive_name: str) -> None:
        archive_url = f"{MUSL_CROSS_BASE_URL}/{archive_name}.tgz"
        install_dir = MUSL_CROSS_INSTALL_ROOT / archive_name

        if self.args.dry_run:
            self.print_command(["curl", "-fsSLO", archive_url])
            self.print_command(["sudo", "mkdir", "-p", str(MUSL_CROSS_INSTALL_ROOT)])
            self.print_command(
                ["sudo", "tar", "-xzf", f"{archive_name}.tgz", "-C", str(MUSL_CROSS_INSTALL_ROOT)]
            )
            self.print_command(
                ["sudo", "ln", "-sfn", f"{install_dir}/bin/<tool>", "/usr/local/bin/<tool>"]
            )
            return

        if not install_dir.exists():
            with tempfile.TemporaryDirectory(prefix="buckyos-musl-") as temp_dir:
                archive_path = Path(temp_dir) / f"{archive_name}.tgz"
                try:
                    self.download_file(archive_url, archive_path)
                except Exception as error:
                    raise BootstrapError(
                        f"Failed to download musl cross toolchain from {archive_url}: {error}"
                    ) from error

                self.run(self.require_unix_privilege(["mkdir", "-p", str(MUSL_CROSS_INSTALL_ROOT)]))
                self.run(
                    self.require_unix_privilege(
                        ["tar", "-xzf", str(archive_path), "-C", str(MUSL_CROSS_INSTALL_ROOT)]
                    )
                )

        bin_dir = install_dir / "bin"
        if not bin_dir.exists():
            raise BootstrapError(
                f"musl cross toolchain {archive_name} extracted without a bin/ directory at {install_dir}"
            )

        for binary_path in sorted(bin_dir.iterdir()):
            if not (binary_path.is_file() or binary_path.is_symlink()):
                continue
            link_path = Path("/usr/local/bin") / binary_path.name
            self.run(
                self.require_unix_privilege(
                    ["ln", "-sfn", str(binary_path), str(link_path)]
                )
            )

        self.notes.append(f"Installed musl cross toolchain at {install_dir}")

    def check_linux_static_tooling(self) -> None:
        if not shutil.which("musl-gcc") and not shutil.which("x86_64-linux-musl-gcc"):
            self.warnings.append(
                "x86_64 musl toolchain not found; static Linux builds require musl-gcc or x86_64-linux-musl-gcc"
            )

        if not shutil.which("musl-g++") and not shutil.which("x86_64-linux-musl-g++"):
            self.warnings.append(
                "x86_64 musl C++ toolchain not found; crates with C++ deps (for example rocksdb) require "
                "musl-g++ or x86_64-linux-musl-g++. On Ubuntu, musl-tools alone is not enough."
            )

        if (
            not shutil.which("aarch64-linux-musl-gcc")
            and not Path("/usr/aarch64-linux-musl/bin/musl-gcc").exists()
            and not Path("/opt/musl-cross/bin/aarch64-linux-musl-gcc").exists()
        ):
            self.warnings.append(
                "aarch64 musl toolchain not found; static Linux builds require aarch64-linux-musl-gcc "
                "or /usr/aarch64-linux-musl/bin/musl-gcc or /opt/musl-cross/bin/aarch64-linux-musl-gcc"
            )

    def check_linux_bindgen_tooling(self) -> None:
        if not shutil.which("clang"):
            self.warnings.append("clang not found; crates using bindgen or C/C++ build steps may fail")

        if not self.find_libclang():
            self.warnings.append(
                "libclang not found; crates using bindgen may fail unless LIBCLANG_PATH points to libclang.so"
            )

    def ensure_macos_build_tools(self) -> None:
        if self.probe(["xcode-select", "-p"]):
            return
        self.warnings.append("Xcode Command Line Tools not detected; please run `xcode-select --install` first")

    def ensure_rust_toolchain(self) -> None:
        rustup = self.find_rustup()
        if not rustup:
            if self.system != "Windows":
                self.warnings.append("rustup not found; you may run the official install script manually later")
            else:
                self.warnings.append("rustup.exe not found; please reopen terminal and run `rustup default stable`")
            return

        self.run([rustup, "default", "stable"])
        if self.system == "Linux" and not self.args.skip_cross_tools:
            self.run([rustup, "target", "add", "x86_64-unknown-linux-musl"])
            self.run([rustup, "target", "add", "aarch64-unknown-linux-gnu"])
            self.run([rustup, "target", "add", "aarch64-unknown-linux-musl"])
        if self.system == "Darwin" and not self.args.skip_cross_tools:
            self.run([rustup, "target", "add", "x86_64-unknown-linux-musl"])
            self.run([rustup, "target", "add", "aarch64-unknown-linux-musl"])
        if self.system == "Windows" and not self.args.skip_msvc:
            host_arch = platform.machine().lower()
            if host_arch in {"amd64", "x86_64"}:
                self.run([rustup, "target", "add", "x86_64-pc-windows-msvc"])
            elif host_arch in {"arm64", "aarch64"}:
                self.run([rustup, "target", "add", "aarch64-pc-windows-msvc"])

    def find_rustup(self) -> str | None:
        if path := shutil.which("rustup"):
            return path

        candidates = [
            self.invoking_user_home() / ".cargo" / "bin" / "rustup",
            self.invoking_user_home() / ".cargo" / "bin" / "rustup.exe",
            Path.home() / ".cargo" / "bin" / "rustup",
            Path.home() / ".cargo" / "bin" / "rustup.exe",
            Path("/opt/homebrew/bin/rustup"),
            Path("/usr/local/bin/rustup"),
            Path("/home/linuxbrew/.linuxbrew/bin/rustup"),
        ]

        if self.system == "Darwin":
            prefix = self.capture_text(["brew", "--prefix", "rustup"])
            if prefix:
                candidates.insert(0, Path(prefix) / "bin" / "rustup")

        if self.system == "Windows":
            userprofile = os.environ.get("USERPROFILE")
            if userprofile:
                candidates.insert(0, Path(userprofile) / ".cargo" / "bin" / "rustup.exe")

        for candidate in candidates:
            if candidate.exists():
                return str(candidate)
        return None

    def ensure_buckyos_directory(self) -> None:
        if self.system == "Windows":
            return

        target = Path("/opt/buckyos")
        owner = os.environ.get("SUDO_USER") or os.environ.get("USER")
        group_name = None

        if owner:
            try:
                group_name = grp.getgrgid(pwd.getpwnam(owner).pw_gid).gr_name
            except KeyError:
                group_name = None

        if not target.exists():
            command = ["mkdir", "-p", str(target)]
            self.run(self.require_unix_privilege(command))

        if owner and group_name:
            self.run(self.require_unix_privilege(["chown", "-R", f"{owner}:{group_name}", str(target)]))
        self.notes.append(f"Prepared {target}")

    def bootstrap(self) -> None:
        print(f"Detected platform: {self.system} ({self.package_manager})")
        if self.args.dry_run:
            print("Dry run enabled: commands will be printed but not executed")

        if self.system == "Linux":
            self.install_linux_environment()
        elif self.system == "Darwin":
            self.install_macos_environment()
        elif self.system == "Windows":
            self.install_windows_environment()
        else:
            raise BootstrapError(f"Unsupported system: {self.system}")

        self.print_summary()

    def print_summary(self) -> None:
        print("\nEnvironment bootstrap completed.")
        print("\nInstalled or checked:")
        print("- Rust toolchain (stable)")
        print("- Node.js + pnpm")
        print("- Python 3")
        print("- uv")
        print("- Deno")
        if self.system != "Windows":
            print("- tmux")
        if not self.args.skip_docker:
            print("- Docker / Docker Desktop")
        if self.system == "Linux" and not self.args.skip_cross_tools:
            print("- Linux cross-compilation helpers (best effort)")
        if self.system != "Windows" and not self.args.skip_buckyos_dir:
            print("- /opt/buckyos")

        if self.notes:
            print("\nNotes:")
            for note in self.notes:
                print(f"- {note}")

        if self.warnings:
            print("\nWarnings:")
            for warning in self.warnings:
                print(f"- {warning}")

        print("\nNext steps:")
        if self.system == "Windows":
            print("- Reopen terminal to ensure winget-installed software is in PATH")
            print("- `cd buckyos`")
            print("- `uv run src\\buckyos-build.py --no-build-web-apps`")
        else:
            print("- Reopen terminal to ensure rustup/uv/deno are in PATH")
            print("- `cd buckyos`")
            print("- `uv run build.py`")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Bootstrap a BuckyOS development environment")
    parser.add_argument("--dry-run", action="store_true", help="Only print commands to be executed")
    parser.add_argument("--skip-docker", action="store_true", help="Skip Docker / Docker Desktop")
    parser.add_argument("--skip-cross-tools", action="store_true", help="Skip Linux cross-compilation dependencies")
    parser.add_argument("--skip-buckyos-dir", action="store_true", help="Skip /opt/buckyos directory preparation")
    parser.add_argument("--skip-msvc", action="store_true", help="Skip Visual Studio Build Tools on Windows")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    bootstrapper = Bootstrapper(args)
    bootstrapper.bootstrap()


if __name__ == "__main__":
    try:
        main()
    except BootstrapError as error:
        print(f"Error: {error}", file=sys.stderr)
        sys.exit(1)
