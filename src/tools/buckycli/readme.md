# buckycli

## 基本操作

```
buckycli --version
```

## 管理DID(账号管理)
命令行的钱包

## 操作SystemConfig
buckycli connect 
buckycli set_sys
buckycli get_sys


## 管理pkg

buckycli pack_pkg $src_dir $target_dir # 将本地目录打包，可以跳过签名
buckycli pub_pkg $target_dir --pkg_name $pkg_name # 发布pkg到repo的待发布index
buckycli pub_app $target_dir # 发布app到repo的待发布index,注意
buckycli repo_publish # 
buckycli install pkg_name  # 这个需要在env目录运行
buckycli publish_app $remote_repo_host #发布app到另一个repo
buckycli install_app $app_name --config1 v1 --config2 v2  # zone级别的安装




