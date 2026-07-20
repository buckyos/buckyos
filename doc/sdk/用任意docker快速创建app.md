# 用任意docker快速创建buckyos dapp

输入信息： docker-url / app_name / 基本配置
```json
{
    "did":"did:bns:username-appname.owner",
    "doc_type":"app",
    "name":"username-appname",
    "version":"0.1.0",
    "tag":"latest",
    "show_name" : "appname",
    "description" : {
        "detail":"appname"
    },
    "author" : "did:web:user-zone-host",
    "owner" : "did:bns:owner",
    "categories": ["dapp"],
    "selector_type": "single",
    "exp":0,
    "pkg_list" : {
        "amd64_docker_image" : {
            "docker_image_name":"docker-url",
            "pkg_id":"username-appname-img#0.1.0"
        }
    },
    "deps":{

    },
    "service_config_tips": {
        "data_mount_points": {},
        "local_cache_mount_points": {},
        "service_endpoints": {
            "www": {
                "protocol": "http",
                "inner_port": 80,
                "required": true,
                "expose": {
                    "route": {
                        "type": "web"
                    }
                }
            }
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
