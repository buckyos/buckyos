# key url Zone提供的一些重要的路径


## 发布的内容
http://$zone_hostname/pub/$named_mgr/path (GET)
http://$zone_hostname/pub/repo/meta_index.db (GET FileObj)
http://$zone_hostname/pub/repo/meta_index.db/content (GET FileObj.content)
http://$zone_hostname/pub/repo/pkg/$pkg_name/$version/chunk (GET)

## zone内的标准路径
http://$zone_hostname/ndn/$chunkid (GET | HEAD | PUT/PATCH)


## default repo (zone内)
http://$zone_hostname/ndn/repo/meta_index.db
http://$zone_hostname/ndn/repo/meta_index.db/content

## my-pub repo (zone外), 可能未签名
http://$zone_hostname/ndn/repo/pub_meta_index.db








