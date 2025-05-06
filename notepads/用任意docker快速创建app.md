# 用任意docker快速创建buckyos dapp

输入信息： docker-url / app_name / 基本配置
```json
{
    "pkg_name":"username-appname",
    "version":"0.1.0",
    "tag":"latest",
    "app_name" : "appname",
    "description" : {
        "detail":"appname"
    },
    "author" : "user_zone_host",
    "pub_time":0,
    "exp":0,
    "pkg_list" : {
        "amd64_docker_image" : {
            "docker_image_name":"docker-url"
        },
    },
    "deps":{

    },
    "install_config" : {
        "data_mount_point" : [],
        "cache_mount_point" : [],
        "local_cache_mount_point" : [],
        "tcp_ports" : {
            "www":80
        },
        "udp_ports" : {
        }
    }
}
```



是否需要构造tar?
    是则下载docker image并导出tar
    更新docker_image_name


## 基本配置（权限需求）
设置Enable的服务
    HTTP
    TCP任何服务

设置docker的需要map的volume
