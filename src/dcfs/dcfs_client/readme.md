# DCFS的基本设计

## smb:// client
最常见的Client，拥有最广泛的兼容性
原理上连接任意Cache Server提供的SMB服务都可以工作
使用Zone内DNS可以更好的选择CacheServer

## FUSE Client
在Linux上将DCFS挂载成本地FS，对类似K8S的基础服务非常重要
FUSE Client在实现时依赖本地的DCFS Client。但可以设备情况DCFS Client进行配置（比如是一个小型设备，那么DCFS Client的Local Cache会非常小）

## DCFS Client
可配置项目
- Cache大小
- 是否只使用指定的Cache Server （通常是Local 优先）

## Cache Server

## Chunk Server

## Meta Server