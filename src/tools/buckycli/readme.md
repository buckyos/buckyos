# buckycli

## Basic Operations

```
buckycli --version
```

## Manage DID (Account Management)
Command line wallet

## SystemConfig Operations
buckycli connect 
buckycli set_sys
buckycli get_sys


## Manage Packages

buckycli pack_pkg $src_dir $target_dir # Pack local directory, can skip signing
buckycli pub_pkg $target_dir --pkg_name $pkg_name # Publish pkg to repo's pending publish index
buckycli pub_app $target_dir # Publish app to repo's pending publish index, note
buckycli repo_publish # 
buckycli install pkg_name  # This needs to run in env directory
buckycli publish_app $remote_repo_host # Publish app to another repo
buckycli install_app $app_name --config1 v1 --config2 v2  # Zone-level installation




