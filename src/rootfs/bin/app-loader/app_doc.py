"""
Python implementation for parsing BuckyOS AppDoc JSON.

This module is intended to mirror the core shape of:
`src/kernel/buckyos-api/src/app_doc.rs`

Design goals (for app-loader usage):
- Robust parsing with reasonable defaults (tolerant to older/variant schemas).
- Provide helpers for selecting docker image by current host arch.
- Keep raw dict for forward compatibility.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Dict, List, Mapping, Optional, Tuple

import platform


class AppDocError(Exception):
    pass


class AppType(str, Enum):
    SERVICE = "service"
    APP_SERVICE = "dapp"
    WEB = "web"
    AGENT = "agent"

    @classmethod
    def from_str(cls, value: str) -> "AppType":
        v = (value or "").strip().lower()
        if v == "service":
            return cls.SERVICE
        if v == "dapp":
            return cls.APP_SERVICE
        if v == "web":
            return cls.WEB
        if v == "agent":
            return cls.AGENT
        raise AppDocError(f"Invalid app doc type: {value!r}")


class SelectorType(str, Enum):
    SINGLE = "single"
    STATIC = "static"
    RANDOM = "random"
    BY_EVENT = "by_event"

    @classmethod
    def from_str(cls, value: str) -> "SelectorType":
        v = (value or "").strip().lower()
        if v in ("single",):
            return cls.SINGLE
        if v in ("static",):
            return cls.STATIC
        if v in ("random",):
            return cls.RANDOM
        if v in ("by_event",):
            return cls.BY_EVENT
        # Rust side treats unknown values as custom selector types.
        # app-loader does not currently depend on custom behaviors,
        # so we keep the string via a best-effort fallback.
        return cls.SINGLE


def _as_dict(v: Any) -> Dict[str, Any]:
    return v if isinstance(v, dict) else {}


def _as_list(v: Any) -> List[Any]:
    return v if isinstance(v, list) else []


def _as_str(v: Any) -> Optional[str]:
    if v is None:
        return None
    if isinstance(v, str):
        return v
    return str(v)


def _as_int(v: Any) -> Optional[int]:
    try:
        if v is None:
            return None
        return int(v)
    except Exception:
        return None


@dataclass(frozen=True)
class SubPkgDesc:
    pkg_id: str
    docker_image_name: Optional[str] = None
    docker_image_digest: Optional[str] = None
    source_url: Optional[str] = None
    pkg_objid: Optional[str] = None
    raw: Dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_dict(cls, d: Mapping[str, Any]) -> "SubPkgDesc":
        dd = dict(d or {})
        pkg_id = _as_str(dd.get("pkg_id")) or ""
        if not pkg_id:
            raise AppDocError("SubPkgDesc missing required field: pkg_id")
        return cls(
            pkg_id=pkg_id,
            pkg_objid=_as_str(dd.get("pkg_objid")),
            docker_image_name=_as_str(dd.get("docker_image_name")),
            docker_image_digest=_as_str(dd.get("docker_image_digest")),
            source_url=_as_str(dd.get("source_url")),
            raw=dd,
        )


@dataclass(frozen=True)
class SubPkgList:
    amd64_docker_image: Optional[SubPkgDesc] = None
    aarch64_docker_image: Optional[SubPkgDesc] = None
    amd64_win_app: Optional[SubPkgDesc] = None
    aarch64_win_app: Optional[SubPkgDesc] = None
    aarch64_apple_app: Optional[SubPkgDesc] = None
    amd64_apple_app: Optional[SubPkgDesc] = None
    web: Optional[SubPkgDesc] = None
    others: Dict[str, SubPkgDesc] = field(default_factory=dict)
    raw: Dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_dict(cls, d: Mapping[str, Any]) -> "SubPkgList":
        dd = dict(d or {})

        def parse_optional(key: str) -> Optional[SubPkgDesc]:
            v = dd.get(key)
            if v is None:
                return None
            if not isinstance(v, dict):
                # Some schemas may use null or unexpected types.
                return None
            return SubPkgDesc.from_dict(v)

        known_keys = {
            "amd64_docker_image",
            "aarch64_docker_image",
            "amd64_win_app",
            "aarch64_win_app",
            "aarch64_apple_app",
            "amd64_apple_app",
            "web",
        }

        others: Dict[str, SubPkgDesc] = {}
        for k, v in dd.items():
            if k in known_keys:
                continue
            if isinstance(v, dict) and v.get("pkg_id"):
                try:
                    others[k] = SubPkgDesc.from_dict(v)
                except Exception:
                    # Ignore unknown/broken entries to keep app-loader resilient.
                    pass

        return cls(
            amd64_docker_image=parse_optional("amd64_docker_image"),
            aarch64_docker_image=parse_optional("aarch64_docker_image"),
            amd64_win_app=parse_optional("amd64_win_app"),
            aarch64_win_app=parse_optional("aarch64_win_app"),
            aarch64_apple_app=parse_optional("aarch64_apple_app"),
            amd64_apple_app=parse_optional("amd64_apple_app"),
            web=parse_optional("web"),
            others=others,
            raw=dd,
        )

    def get(self, key: str) -> Optional[SubPkgDesc]:
        if key == "amd64_docker_image":
            return self.amd64_docker_image
        if key == "aarch64_docker_image":
            return self.aarch64_docker_image
        if key == "amd64_win_app":
            return self.amd64_win_app
        if key == "aarch64_win_app":
            return self.aarch64_win_app
        if key == "aarch64_apple_app":
            return self.aarch64_apple_app
        if key == "amd64_apple_app":
            return self.amd64_apple_app
        if key == "web":
            return self.web
        return self.others.get(key)

    def get_docker_image_for_host(self, machine: Optional[str] = None) -> SubPkgDesc:
        m = (machine or platform.machine() or "").lower()
        # Common values:
        # - x86_64 (mac/linux)
        # - amd64 (docker/cli)
        # - arm64 (mac)
        # - aarch64 (linux)
        if m in ("x86_64", "amd64", "x64"):
            if self.amd64_docker_image is None:
                raise AppDocError("No amd64 docker image in pkg_list")
            return self.amd64_docker_image
        if m in ("arm64", "aarch64"):
            if self.aarch64_docker_image is None:
                raise AppDocError("No aarch64 docker image in pkg_list")
            return self.aarch64_docker_image

        # Fallback preference: amd64 then aarch64.
        if self.amd64_docker_image is not None:
            return self.amd64_docker_image
        if self.aarch64_docker_image is not None:
            return self.aarch64_docker_image
        raise AppDocError(f"No docker image available for host machine={machine!r}")


@dataclass(frozen=True)
class ServiceInstallConfigTips:
    service_ports: Dict[str, int] = field(default_factory=dict)
    data_mount_point: List[str] = field(default_factory=list)
    local_cache_mount_point: List[str] = field(default_factory=list)
    container_param: Optional[str] = None
    start_param: Optional[str] = None
    custom_config: Dict[str, Any] = field(default_factory=dict)
    raw: Dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_dict(cls, d: Mapping[str, Any]) -> "ServiceInstallConfigTips":
        dd = dict(d or {})
        ports_raw = dd.get("service_ports")
        ports: Dict[str, int] = {}
        if isinstance(ports_raw, dict):
            for k, v in ports_raw.items():
                iv = _as_int(v)
                if iv is not None:
                    ports[str(k)] = iv

        # Rust uses `flatten` for custom_config; keep unknown keys too.
        known_keys = {
            "service_ports",
            "custom_service_desc",
            "data_mount_point",
            "data_mount_recommend",
            "local_cache_mount_point",
            "container_param",
            "start_param",
        }
        custom_config = {k: v for k, v in dd.items() if k not in known_keys}

        return cls(
            service_ports=ports,
            data_mount_point=[_as_str(x) or "" for x in _as_list(dd.get("data_mount_point")) if _as_str(x)],
            local_cache_mount_point=[
                _as_str(x) or "" for x in _as_list(dd.get("local_cache_mount_point")) if _as_str(x)
            ],
            container_param=_as_str(dd.get("container_param")),
            start_param=_as_str(dd.get("start_param")),
            custom_config=custom_config,
            raw=dd,
        )


@dataclass(frozen=True)
class AppDoc:
    # PackageMeta flattened fields (subset used by app-loader; keep raw for rest).
    name: str
    version: str
    show_name: str
    author: Optional[str] = None
    owner: Optional[str] = None
    tag: Optional[str] = None
    categories: List[str] = field(default_factory=list)
    selector_type: SelectorType = SelectorType.SINGLE
    install_config_tips: ServiceInstallConfigTips = field(default_factory=ServiceInstallConfigTips)
    pkg_list: SubPkgList = field(default_factory=SubPkgList)
    raw: Dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_dict(cls, d: Mapping[str, Any]) -> "AppDoc":
        dd = dict(d or {})

        name = _as_str(dd.get("name")) or ""
        version = _as_str(dd.get("version")) or ""
        if not name or not version:
            raise AppDocError(f"Invalid AppDoc: missing name/version (name={name!r}, version={version!r})")

        show_name = _as_str(dd.get("show_name")) or name
        categories = [str(x) for x in _as_list(dd.get("categories")) if _as_str(x)]

        selector_type = SelectorType.from_str(_as_str(dd.get("selector_type")) or "single")
        install_config_tips = ServiceInstallConfigTips.from_dict(_as_dict(dd.get("install_config_tips")))
        pkg_list = SubPkgList.from_dict(_as_dict(dd.get("pkg_list")))

        return cls(
            name=name,
            version=version,
            show_name=show_name,
            author=_as_str(dd.get("author")),
            owner=_as_str(dd.get("owner")),
            tag=_as_str(dd.get("tag")) or _as_str(dd.get("version_tag")),
            categories=categories,
            selector_type=selector_type,
            install_config_tips=install_config_tips,
            pkg_list=pkg_list,
            raw=dd,
        )

    def get_app_type(self) -> AppType:
        if self.categories:
            try:
                return AppType.from_str(self.categories[0])
            except Exception:
                return AppType.SERVICE
        return AppType.SERVICE

    def get_docker_image_for_host(self, machine: Optional[str] = None) -> Tuple[str, str]:
        """
        Returns (docker_image_name, pkg_id) for current host arch.
        """
        desc = self.pkg_list.get_docker_image_for_host(machine=machine)
        if not desc.docker_image_name:
            raise AppDocError("docker_image_name missing in selected docker image sub-package")
        return desc.docker_image_name, desc.pkg_id

