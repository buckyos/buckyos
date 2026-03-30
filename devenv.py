#!/usr/bin/env python3
from __future__ import annotations

import argparse
from dataclasses import dataclass, field
import os
from pathlib import Path
import platform
import shlex
import shutil
import subprocess
import sys
from typing import Callable, Sequence

if os.name != "nt":
    import grp
    import pwd


class BootstrapError(RuntimeError):
    pass


LINUX_CORE_PACKAGES = {
    "apt-get": ["build-essential", "curl", "wget", "git", "pkg-config", "libssl-dev"],
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

LINUX_NODE_CHOICES = {
    "apt-get": [["nodejs", "npm"]],
    "dnf": [["nodejs", "npm"], ["nodejs"]],
    "yum": [["nodejs", "npm"], ["nodejs"]],
    "pacman": [["nodejs", "npm"]],
    "zypper": [["nodejs22", "npm-default"], ["nodejs20", "npm-default"], ["nodejs", "npm-default"], ["nodejs", "npm"]],
}

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

    def find_binary(self, binary_name: str, candidates: Sequence[Path]) -> str | None:
        if path := shutil.which(binary_name):
            return path

        for candidate in candidates:
            if candidate.exists():
                return str(candidate)
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

    def ensure_tmux(self) -> None:
        if shutil.which("tmux"):
            return

        if self.system == "Windows":
            self.warnings.append("tmux is not installed automatically on native Windows; use WSL2 if you need tmux")
            return

        if self.system == "Linux":
            self.install_first_resolved_set("tmux", LINUX_TMUX_CHOICES[self.package_manager])

    def install_linux_environment(self) -> None:
        self.update_package_index()
        self.install_packages(LINUX_CORE_PACKAGES[self.package_manager])
        self.install_first_resolved_set("Python 3", LINUX_PYTHON_CHOICES[self.package_manager])
        self.install_first_resolved_set("Node.js", LINUX_NODE_CHOICES[self.package_manager])
        self.install_first_resolved_set("rustup", LINUX_RUSTUP_CHOICES[self.package_manager])
        self.ensure_uv()
        self.ensure_deno()
        self.ensure_tmux()

        if not self.args.skip_docker:
            self.install_first_resolved_set("Docker", LINUX_DOCKER_CHOICES[self.package_manager], optional=True)

        if not shutil.which("pnpm"):
            package_set = self.install_first_resolved_set(
                "pnpm",
                LINUX_PNPM_CHOICES[self.package_manager],
                optional=True,
            )
            if package_set is None and shutil.which("npm"):
                self.run(self.require_privilege(["npm", "install", "-g", "pnpm"]))
            elif package_set is None:
                self.warnings.append("Node.js is installed but pnpm was not found; please install manually")

        self.ensure_rust_toolchain()

        if not self.args.skip_cross_tools:
            for choices in LINUX_CROSS_PACKAGE_CHOICES[self.package_manager]:
                self.install_first_resolved_set("cross-compilation dependencies", [choices], optional=True)
            self.check_linux_static_tooling()

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

        if not self.args.skip_buckyos_dir:
            self.ensure_buckyos_directory()

        rustup_prefix = self.capture_text(["brew", "--prefix", "rustup"])
        if rustup_prefix:
            self.notes.append(f"If rustup is not found in terminal, add {rustup_prefix}/bin to PATH")

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

    def check_linux_static_tooling(self) -> None:
        if not shutil.which("musl-gcc") and not shutil.which("x86_64-linux-musl-gcc"):
            self.warnings.append(
                "x86_64 musl toolchain not found; static Linux builds require musl-gcc or x86_64-linux-musl-gcc"
            )

        if not shutil.which("aarch64-linux-musl-gcc") and not Path("/usr/aarch64-linux-musl/bin/musl-gcc").exists():
            self.warnings.append(
                "aarch64 musl toolchain not found; static Linux builds require aarch64-linux-musl-gcc "
                "or /usr/aarch64-linux-musl/bin/musl-gcc"
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
            print("- `uv run src/buckyos-build.py --no-build-web-apps`")


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
