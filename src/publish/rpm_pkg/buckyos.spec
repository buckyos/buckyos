# Rendered by make_local_rpm.py. Keep placeholders in double-brace form.
Name: buckyos
Version: {{rpm_version}}
Release: {{rpm_release}}
Summary: BuckyOS system software
License: Proprietary
URL: https://buckyos.org
AutoReqProv: no

Requires: python3
Requires: curl
Requires: openssl
Requires: psmisc
Requires: (moby-engine or docker-ce or docker-engine)

%global debug_package %{nil}
# Keep prebuilt payload files unchanged after %install.
%global __os_install_post %{nil}
%global _build_id_links none

%description
BuckyOS system software, including node_daemon, node_active, cyfs_gateway,
app_loader, system_config_service, verify_hub and default config files.

%prep

%build

%install
rm -rf "%{buildroot}"
mkdir -p "%{buildroot}"
cp -a {{payload_tree}}/. "%{buildroot}/"

%pre
{{pre_script}}
%post
{{post_script}}
%preun
{{preun_script}}
%postun
{{postun_script}}
%files
%defattr(-,root,root,-)
/opt/buckyos
