# cyfs-sn的主要功能

1. 提供基于http api的设备注册（IP地址更新）
2. 提供基于http api的设备注销
3. 提供基于http api的设备查询
4. 提供内部接口，cyfs-dns打开时，可在查询 $device_id.d.baseurl时，返回设备的IP地址，并根据来源确定返回外网还是内网地址
