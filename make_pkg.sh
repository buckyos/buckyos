#! /bin/bash

source ./venv/bin/activate
rm -rf /opt/buckyosci/buckyos/
rm -rf /opt/buckyosci/buckycli/
cd ../cyfs-gateway/src && cargo update && buckyos-build && buckyos-install --all --target-rootfs=/opt/buckyosci/buckyos --app=cyfs-gateway
cd ../../buckyos/src && cargo update && buckyos-build && buckyos-install --all --target-rootfs=/opt/buckyosci/buckycli --app=buckycli && buckyos-install --all --target-rootfs=/opt/buckyosci/buckyos --app=buckyos && python3 make_config.py release --rootfs=/opt/buckyosci/buckyos
cd .. && python3 ./src/publish/make_local_osx_pkg.py build-pkg aarch64 0.5.1+build260115