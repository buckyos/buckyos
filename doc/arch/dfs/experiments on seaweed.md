# 实验环境

multipass Ubuntu 虚拟机，挂载在ssd上
跑master
weed master -mdir="."

wsl Ubuntu， 挂载在ssd上
我的机器有4块硬盘，2*ssd + 2*hdd
c盘和f盘是ssd
d盘和e盘是hdd 

```
## ssd1
AS SSD Benchmark 2.0.7316.34247
------------------------------
Name: Samsung SSD 970 EVO Plus 250GB
Firmware: 1B2QEXM7
Controller: stornvme
Offset: 233472 K - OK
Size: 232.88 GB
Date: 2024/11/28 14:18:03
------------------------------
Sequential:
------------------------------
Read: 2863.65 MB/s
Write: 2127.69 MB/s
------------------------------
4K:
------------------------------
Read: 49.57 MB/s
Write: 89.19 MB/s
------------------------------
4K-64Threads:
------------------------------
Read: 790.21 MB/s
Write: 1066.48 MB/s
------------------------------
Access Times:
------------------------------
Read: 0.090 ms
Write: 0.046 ms
------------------------------
Score:
------------------------------
Read: 1126
Write: 1368
Total: 3046
------------------------------
```

## ssd2
```
AS SSD Benchmark 2.0.7316.34247
------------------------------
Name: KINGSTON SNVS1000GB
Firmware: S8442101
Controller: stornvme
Offset: 16384 K - OK
Size: 931.51 GB
Date: 2024/11/28 14:22:44
------------------------------
Sequential:
------------------------------
Read: 1317.24 MB/s
Write: 1191.53 MB/s
------------------------------
4K:
------------------------------
Read: 34.46 MB/s
Write: 84.51 MB/s
------------------------------
4K-64Threads:
------------------------------
Read: 621.22 MB/s
Write: 571.94 MB/s
------------------------------
Access Times:
------------------------------
Read: 0.703 ms
Write: 0.123 ms
------------------------------
Score:
------------------------------
Read: 787
Write: 776
Total: 1968
------------------------------
```

## hdd1
```
AS SSD Benchmark 2.0.7316.34247
------------------------------
Name: WDC WD20EARS-00MVWB0
Firmware: 51.0AB51
Controller: storahci
Offset: 132096 K - OK
Size: 1863.01 GB
Date: 2024/11/28 14:20:47
------------------------------
Sequential:
------------------------------
Read: 72.95 MB/s
Write: 69.94 MB/s
------------------------------
4K:
------------------------------
Read: 0.00 MB/s
Write: 0.00 MB/s
------------------------------
4K-64Threads:
------------------------------
Read: 0.00 MB/s
Write: 0.00 MB/s
------------------------------
Access Times:
------------------------------
Read: 0.090 ms
Write: 0.046 ms
```



# seaweed shell benchmark 没有经过fuse直接读写

## 挂一个ssd，两个hdd
```
./weed volume -mserver=172.24.202.140:9333 -disk=ssd,hdd,hdd -dir=./data,/mnt/d/seaweed,/mnt/e/seaweed
```


### 读写测试 100K个1KB文件，写到hdd上
```
weed benchmark -n=100000
This is SeaweedFS version 30GB 3.80 7b3c0e937f83d3b49799b5d5dcb98b0043461c25 linux amd64
I1128 10:37:23.668311 masterclient.go:228 master localhost:9333 redirected to leader 172.24.202.140:9333
.
------------ Writing Benchmark ----------


Concurrency Level:      16
Time taken for tests:   19.008 seconds
Completed requests:      100000
Failed requests:        0
Total transferred:      105549233 bytes
Requests per second:    5260.83 [#/sec]
Transfer rate:          5422.62 [Kbytes/sec]

Connection Times (ms)
              min      avg        max      std
Total:        0.4      3.0       273.4      4.5

Percentage of the requests served within a certain time (ms)
   50%      2.7 ms
   66%      3.0 ms
   75%      3.3 ms
   80%      3.5 ms
   90%      4.1 ms
   95%      4.9 ms
   98%      6.0 ms
   99%      6.9 ms
  100%    273.4 ms

------------ Randomly Reading Benchmark ----------


Concurrency Level:      16
Time taken for tests:   7.780 seconds
Completed requests:      100000
Failed requests:        0
Total transferred:      105544046 bytes
Requests per second:    12854.21 [#/sec]
Transfer rate:          13248.88 [Kbytes/sec]

Connection Times (ms)
              min      avg        max      std
Total:        0.2      1.0       11.1      0.5

Percentage of the requests served within a certain time (ms)
   50%      0.9 ms
   66%      1.1 ms
   75%      1.2 ms
   80%      1.3 ms
   90%      1.6 ms
   95%      1.9 ms
   98%      2.4 ms
   99%      2.8 ms
  100%     11.1 ms
```

### 读写测试 100K个1KB文件，写到ssd上
```
weed benchmark -disk=ssd -n=100000
This is SeaweedFS version 30GB 3.80 7b3c0e937f83d3b49799b5d5dcb98b0043461c25 linux amd64
I1128 10:09:38.332211 masterclient.go:228 master localhost:9333 redirected to leader 172.24.202.140:9333
.
------------ Writing Benchmark ----------


Concurrency Level:      16
Time taken for tests:   18.753 seconds
Completed requests:      100000
Failed requests:        0
Total transferred:      105555873 bytes
Requests per second:    5332.56 [#/sec]
Transfer rate:          5496.90 [Kbytes/sec]

Connection Times (ms)
              min      avg        max      std
Total:        0.7      2.9       212.7      2.9

Percentage of the requests served within a certain time (ms)
   50%      2.7 ms
   66%      3.0 ms
   75%      3.3 ms
   80%      3.5 ms
   90%      4.2 ms
   95%      5.0 ms
   98%      6.1 ms
   99%      6.8 ms
  100%    212.7 ms

------------ Randomly Reading Benchmark ----------


Concurrency Level:      16
Time taken for tests:   8.006 seconds
Completed requests:      100000
Failed requests:        0
Total transferred:      105553101 bytes
Requests per second:    12490.31 [#/sec]
Transfer rate:          12874.91 [Kbytes/sec]

Connection Times (ms)
              min      avg        max      std
Total:        0.2      1.0       10.2      0.5

Percentage of the requests served within a certain time (ms)
   50%      0.9 ms
   66%      1.1 ms
   75%      1.2 ms
   80%      1.3 ms
   90%      1.7 ms
   95%      2.0 ms
   98%      2.4 ms
   99%      2.8 ms
  100%     10.2 ms
```

### 读写测试 10K个1MB文件，写到hdd上
```
weed benchmark -n=10000 -size=1048576
This is SeaweedFS version 30GB 3.80 7b3c0e937f83d3b49799b5d5dcb98b0043461c25 linux amd64
I1128 10:44:55.807884 masterclient.go:228 master localhost:9333 redirected to leader 172.24.202.140:9333
.
------------ Writing Benchmark ----------
Concurrency Level:      16
Time taken for tests:   91.539 seconds
Completed requests:      10000
Failed requests:        0
Total transferred:      10486070465 bytes
Requests per second:    109.24 [#/sec]
Transfer rate:          111868.38 [Kbytes/sec]

Connection Times (ms)
              min      avg        max      std
Total:        11.3      146.2       1705.3      177.9

Percentage of the requests served within a certain time (ms)
   50%     95.7 ms
   66%    116.5 ms
   75%    142.6 ms
   80%    163.3 ms
   90%    245.0 ms
   95%    380.7 ms
   98%    842.6 ms
   99%    1082.9 ms
  100%    1705.3 ms

------------ Randomly Reading Benchmark ----------
Concurrency Level:      16
Time taken for tests:   52.683 seconds
Completed requests:      10000
Failed requests:        0
Total transferred:      10486070203 bytes
Requests per second:    189.81 [#/sec]
Transfer rate:          194375.16 [Kbytes/sec]

Connection Times (ms)
              min      avg        max      std
Total:        7.7      71.3       625.5      34.9

Percentage of the requests served within a certain time (ms)
   50%     66.0 ms
   66%     76.8 ms
   75%     83.4 ms
   80%     88.9 ms
   90%    104.9 ms
   95%    122.9 ms
   98%    149.6 ms
   99%    172.9 ms
  100%    625.5 ms
```

### 读写测试 10K个1MB文件，写到ssd上
```
weed benchmark -n=10000 -size=1048576 -disk=ssd
This is SeaweedFS version 30GB 3.80 7b3c0e937f83d3b49799b5d5dcb98b0043461c25 linux amd64
I1128 14:30:38.059217 masterclient.go:228 master localhost:9333 redirected to leader 172.24.202.140:9333
.
------------ Writing Benchmark ----------


Concurrency Level:      16
Time taken for tests:   61.893 seconds
Completed requests:      10000
Failed requests:        0
Total transferred:      10486075477 bytes
Requests per second:    161.57 [#/sec]
Transfer rate:          165452.39 [Kbytes/sec]

Connection Times (ms)
              min      avg        max      std
Total:        18.1      98.1       1808.1      76.4

Percentage of the requests served within a certain time (ms)
   50%     93.3 ms
   66%    101.1 ms
   75%    105.4 ms
   80%    108.2 ms
   90%    116.8 ms
   95%    127.3 ms
   98%    167.5 ms
   99%    257.8 ms
  100%    1808.1 ms

------------ Randomly Reading Benchmark ----------


Concurrency Level:      16
Time taken for tests:   52.166 seconds
Completed requests:      10000
Failed requests:        0
Total transferred:      10486073763 bytes
Requests per second:    191.70 [#/sec]
Transfer rate:          196302.00 [Kbytes/sec]

Connection Times (ms)
              min      avg        max      std
Total:        8.7      71.1       824.1      36.1

Percentage of the requests served within a certain time (ms)
   50%     65.5 ms
   66%     76.3 ms
   75%     83.9 ms
   80%     89.9 ms
   90%    107.9 ms
   95%    126.4 ms
   98%    152.2 ms
   99%    177.1 ms
  100%    824.1 ms
```

### 结论
+ 这个benchmark的实现简单的生成内存随机数据16线程并发写文件, 经过文件系统的元数据， 结果看上去只能做相对的定性分析， 吞吐量的绝对值并没明显不太符合理性预期
+ 1k小文件读写ssd 和 hdd性能基本一样
+ 1m小文件写ssd 比 hdd 好1.5倍， 读性能基本一样
+ 任务管理器观察到读写过程中，hdd的并发性很高，两块hdd的读写负载都接近100%

## 挂一个ssd 一个hdd

```
./weed volume -mserver=172.24.202.140:9333 -disk=ssd,hdd -dir=./data,/mnt/d/seaweed
```

### 读写测试 10K个1MB文件，写到hdd上
```
weed benchmark -n=10000 -size=1048576
This is SeaweedFS version 30GB 3.80 7b3c0e937f83d3b49799b5d5dcb98b0043461c25 linux amd64
I1128 14:55:10.752350 masterclient.go:228 master localhost:9333 redirected to leader 172.24.202.140:9333
.

------------ Writing Benchmark ----------


Concurrency Level:      16
Time taken for tests:   164.907 seconds
Completed requests:      10000
Failed requests:        0
Total transferred:      10486076800 bytes
Requests per second:    60.64 [#/sec]
Transfer rate:          62097.46 [Kbytes/sec]

Connection Times (ms)
              min      avg        max      std
Total:        11.6      263.6       2100.7      283.1

Percentage of the requests served within a certain time (ms)
   50%    160.9 ms
   66%    255.2 ms
   75%    324.7 ms
   80%    371.9 ms
   90%    525.9 ms
   95%    746.0 ms
   98%    1380.7 ms
   99%    1516.1 ms
  100%    2100.7 ms


------------ Randomly Reading Benchmark ----------

Concurrency Level:      16
Time taken for tests:   54.501 seconds
Completed requests:      10000
Failed requests:        0
Total transferred:      10486076232 bytes
Requests per second:    183.48 [#/sec]
Transfer rate:          187891.31 [Kbytes/sec]

Connection Times (ms)
              min      avg        max      std
Total:        8.9      73.9       903.2      35.4

Percentage of the requests served within a certain time (ms)
   50%     69.3 ms
   66%     81.0 ms
   75%     88.5 ms
   80%     94.0 ms
   90%    110.9 ms
   95%    129.0 ms
   98%    150.7 ms
   99%    170.5 ms
  100%    903.2 ms
```
### 结论
+ ssd比单hdd 写性能好3倍， 读性能差不多


# 通过 filer 和 fuse mout 读写

## 写入到hdd
```
weed filer
weed mount -dir=/opt/seaweed/
```

###同样写入10K个1MB文件，方式是写入经过fuse mount的目录
```
root@DESKTOP-2J0TFE0:/mnt/c/Users/tsukasa# python3 /mnt/f/source/buckyos/doc/dfs/fuse_profile.py 16 1048576 10000 /opt/seaweed/ write
开始测试...
线程数: 16
单文件大小: 1048576 字节
总文件数: 10000
目标目录: /opt/seaweed/
测试模式: write

开始写入文件...
写入进度: 9938/10000 文件 已耗时: 189.1秒 平均速度: 52.54 MB/s

完成写入文件
写入总耗时: 190.15 秒
写入平均速度: 52.59 MB/s
```


```
root@DESKTOP-2J0TFE0:/mnt/c/Users/tsukasa# python3 /mnt/f/source/buckyos/doc/dfs/fuse_profile.py 16 1048576 10000 /opt/seaweed/ read
开始测试...
线程数: 16
单文件大小: 1048576 字节
总文件数: 10000
目标目录: /opt/seaweed/
测试模式: read

开始读取文件...
读取进度: 9972/10000 文件 已耗时: 471.1秒 平均速度: 21.17 MB/s

完成读取文件
读取总耗时: 471.88 秒
读取平均速度: 21.19 MB/s
```


### 对照写入VFS目录
```
root@DESKTOP-2J0TFE0:/mnt/c/Users/tsukasa# python3 /mnt/f/source/buckyos/doc/dfs/fuse_profile.py 16 1048576 10000 /mnt/d/fuse_profile both
开始测试...
线程数: 16
单文件大小: 1048576 字节
总文件数: 10000
目标目录: /mnt/d/fuse_profile
测试模式: both

开始写入文件...
写入进度: 9949/10000 文件 已耗时: 295.3秒 平均速度: 33.69 MB/s

完成写入文件
写入总耗时: 296.12 秒
写入平均速度: 33.77 MB/s
```

```
root@DESKTOP-2J0TFE0:/mnt/c/Users/tsukasa# python3 /mnt/f/source/buckyos/doc/dfs/fuse_profile.py 16 1048576 10000 /mnt/d/fuse_profile read
开始测试...
线程数: 16
单文件大小: 1048576 字节
总文件数: 10000
目标目录: /mnt/d/fuse_profile
测试模式: read

开始读取文件...
读取进度: 1042/10000 文件 已耗时: 88.4秒 平均速度: 11.79 MB/
```

线程数改为1规避单hdd并发读带来的性能问题
```
root@DESKTOP-2J0TFE0:/mnt/c/Users/tsukasa# python3 /mnt/f/source/buckyos/doc/dfs/fuse_profile.py 1 1048576 10000 /mnt/d/
fuse_profile read
开始测试...
线程数: 1
单文件大小: 1048576 字节
总文件数: 10000
目标目录: /mnt/d/fuse_profile
测试模式: read

开始读取文件...
读取进度: 2639/10000 文件 已耗时: 87.1秒 平均速度: 30.28 MB/ss
```

### 结论
+ 写入过程中观察任务管理的磁盘性能，发现写入过程的并发性并不高，两个HDD的写入负载交替上升
+ 经过fuse之后的IO性能相比fs benchmark明显下降
+ 造成在何种差距的可能原因是：fuse lib的性能，filer server的性能（可以通过s3接口测试单独测试filer server性能）
+ 写比读快的原因是？
+ 相比VFS目录，读写性能有都有提升



## 通过fuse删除之后的回收
删除之前写入的所有文件
```
rm -rf /opt/seaweed/*
```
观察volume分布，filer中的入口已经被删除，volume文件并没有释放
```
 DataCenter DefaultDataCenter ssd(volume:0/8 active:0 free:8 remote:0) hdd(volume:7/16 active:7 free:9 remote:0)
    Rack DefaultRack ssd(volume:0/8 active:0 free:8 remote:0) hdd(volume:7/16 active:7 free:9 remote:0)
      DataNode 172.23.156.57:8080 hdd(volume:7/16 active:7 free:9 remote:0) ssd(volume:0/8 active:0 free:8 remote:0)
        Disk hdd(volume:7/16 active:7 free:9 remote:0)
          volume id:74  size:1444809832  file_count:1378  delete_count:1377  deleted_byte_count:1443911035  version:3  compact_revision:1  modified_at_second:1732848872
          volume id:70  size:1518281568  file_count:1448  delete_count:1447  deleted_byte_count:1517312465  version:3  compact_revision:1  modified_at_second:1732848872
          volume id:75  size:1444911440  file_count:1379  delete_count:1377  deleted_byte_count:1443911031  version:3  compact_revision:1  modified_at_second:1732848872
          volume id:71  size:1521427656  file_count:1451  delete_count:1450  deleted_byte_count:1520458246  version:3  compact_revision:1  modified_at_second:1732848872
          volume id:73  size:1492582104  file_count:1423  delete_count:1420  deleted_byte_count:1489000478  version:3  compact_revision:1  modified_at_second:1732848872
          volume id:76  size:1538799216  file_count:1468  delete_count:1467  deleted_byte_count:1538284291  version:3  compact_revision:1  modified_at_second:1732848872
          volume id:72  size:1534888968  file_count:1463  delete_count:1462  deleted_byte_count:1533041344  version:3  compact_revision:1  modified_at_second:1732848881
        Disk hdd total size:10495700784 file_count:10010 deleted_file:10000 deleted_bytes:10485918890
        Disk ssd(volume:0/8 active:0 free:8 remote:0)
        Disk ssd total size:0 file_count:0
      DataNode 172.23.156.57:8080 total size:10495700784 file_count:10010 deleted_file:10000 deleted_bytes:10485918890
    Rack DefaultRack total size:10495700784 file_count:10010 deleted_file:10000 deleted_bytes:10485918890
```

执行 volume.vacc， 一段时间之后volumen中的垃圾数据被释放
```
Topology volumeSizeLimit:30000 MB ssd(volume:0/8 active:0 free:8 remote:0) hdd(volume:7/16 active:7 free:9 remote:0)
  DataCenter DefaultDataCenter ssd(volume:0/8 active:0 free:8 remote:0) hdd(volume:7/16 active:7 free:9 remote:0)
    Rack DefaultRack hdd(volume:7/16 active:7 free:9 remote:0) ssd(volume:0/8 active:0 free:8 remote:0)
      DataNode 172.23.156.57:8080 hdd(volume:7/16 active:7 free:9 remote:0) ssd(volume:0/8 active:0 free:8 remote:0)
        Disk hdd(volume:7/16 active:7 free:9 remote:0)
          volume id:70  size:876336  file_count:1  version:3  compact_revision:2  modified_at_second:1732849188
          volume id:75  size:912128  file_count:2  version:3  compact_revision:2  modified_at_second:1732849198
          volume id:71  size:876456  file_count:1  version:3  compact_revision:2  modified_at_second:1732849168
          volume id:73  size:3490584  file_count:3  version:3  compact_revision:2  modified_at_second:1732849178
          volume id:76  size:420864  file_count:1  version:3  compact_revision:2  modified_at_second:1732849148
          volume id:72  size:1753896  file_count:1  version:3  compact_revision:2  modified_at_second:1732849158
          volume id:74  size:810520  file_count:1  version:3  compact_revision:2  modified_at_second:1732849138
        Disk hdd total size:9140784 file_count:10
        Disk ssd(volume:0/8 active:0 free:8 remote:0)
        Disk ssd total size:0 file_count:0
      DataNode 172.23.156.57:8080 total size:9140784 file_count:10
    Rack DefaultRack total size:9140784 file_count:10
  DataCenter DefaultDataCenter total size:9140784 file_count:10
total size:9140784 file_count:10
```

## 写入到ssd
```
weed filer -defaultStoreDir=/mnt/f/seaweed -disk=ssd -collection=benchm
ark
```
### 写入10K个1MB文件
```
root@DESKTOP-2J0TFE0:/mnt/c/Users/tsukasa# python3 /mnt/f/source/buckyos/doc/dfs/fuse_profile.py 16 1048576 10000 /opt/seaweed/ both
开始测试...
线程数: 16
单文件大小: 1048576 字节
总文件数: 10000
目标目录: /opt/seaweed/
测试模式: both

开始写入文件...
写入进度: 9988/10000 文件 已耗时: 185.1秒 平均速度: 53.96 MB/s

完成写入文件
写入总耗时: 185.24 秒
写入平均速度: 53.98 MB/s

root@DESKTOP-2J0TFE0:/mnt/c/Users/tsukasa# python3 /mnt/f/source/buckyos/doc/dfs/fuse_profile.py 16 1048576 10000 /opt/seaweed/ read
开始测试...
线程数: 16
单文件大小: 1048576 字节
总文件数: 10000
目标目录: /opt/seaweed/
测试模式: read

开始读取文件...
读取进度: 9956/10000 文件 已耗时: 93.2秒 平均速度: 106.82 MB/s

完成读取文件
读取总耗时: 93.68 秒
读取平均速度: 106.74 MB/s
```

### 对照写入VFS目录
```
root@DESKTOP-2J0TFE0:/mnt/c/Users/tsukasa# python3 /mnt/f/source/buckyos/doc/dfs/fuse_profile.py 16 1048576 10000 /mnt/f
/fuse_profile write
开始测试...
线程数: 16
单文件大小: 1048576 字节
总文件数: 10000
目标目录: /mnt/f/fuse_profile
测试模式: write

开始写入文件...
写入进度: 9959/10000 文件 已耗时: 205.2秒 平均速度: 48.54 MB/s


root@DESKTOP-2J0TFE0:/mnt/c/Users/tsukasa# python3 /mnt/f/source/buckyos/doc/dfs/fuse_profile.py 16 1048576 10000 /mnt/f/fuse_profile read
开始测试...
线程数: 16
单文件大小: 1048576 字节
总文件数: 10000
目标目录: /mnt/f/fuse_profile
测试模式: read

开始读取文件...
读取进度: 9845/10000 文件 已耗时: 33.2秒 平均速度: 296.74 MB/s
```

### 结论
+ 写入到ssd的性能比hdd好很多
+ 还是远远没有到ssd的实际IO峰值
+ 相比VFS 写入性能基本一致，读性能只有三分之一



# 把ssd当作写缓存

## 配置所有写入指向ssd磁盘
```
> fs.configure -locationPrefix=/ -collection=benchmark -disk=ssd -volumeGrowthCount=1 -apply
{
  "version":  0,
  "locations":  [
    {
      "locationPrefix":  "/",
      "collection":  "benchmark",
      "replication":  "",
      "ttl":  "",
      "diskType":  "ssd",
      "fsync":  false,
      "volumeGrowthCount":  1,
      "readOnly":  false,
      "dataCenter":  "",
      "rack":  "",
      "dataNode":  "",
      "maxFileNameLength":  0,
      "disableChunkDeletion":  false,
      "worm":  false
    }
  ]
}
```
通过配置fs所有写入指向ssd, 并且设置volumeGrowthCount=1， 因为只有1个ssd，单volume写入并不会因为不能并发导致明显性能下降；通过fuse profile测试的结果也跟默认volumeGrowthCount=7的性能基本一致；

可以看到新写入的数据都集中在ssd上的单个volume id=4中
```
Topology volumeSizeLimit:30000 MB hdd(volume:3/16 active:3 free:13 remote:0) ssd(volume:1/8 active:1 free:7 remote:0)
  DataCenter DefaultDataCenter hdd(volume:3/16 active:3 free:13 remote:0) ssd(volume:1/8 active:1 free:7 remote:0)
    Rack DefaultRack ssd(volume:1/8 active:1 free:7 remote:0) hdd(volume:3/16 active:3 free:13 remote:0)
      DataNode 172.23.156.57:8080 ssd(volume:1/8 active:1 free:7 remote:0)
        Disk ssd(volume:1/8 active:1 free:7 remote:0)
          volume id:4  size:1267786424  file_count:1209  version:3  modified_at_second:1732867272  disk_type:"ssd"
        Disk ssd total size:1267786424 file_count:1209
      DataNode 172.23.156.57:8080 total size:1267786424 file_count:1209
      DataNode 172.23.156.57:8081 hdd(volume:3/16 active:3 free:13 remote:0)
        Disk hdd(volume:3/16 active:3 free:13 remote:0)
          volume id:3  size:8  version:3  modified_at_second:1732866801
          volume id:1  size:24592  file_count:3  version:3  compact_revision:1  modified_at_second:1732867243
          volume id:2  size:8  version:3  modified_at_second:1732866800
        Disk hdd total size:24608 file_count:3
      DataNode 172.23.156.57:8081 total size:24608 file_count:3
    Rack DefaultRack total size:1267811032 file_count:1212
  DataCenter DefaultDataCenter total size:1267811032 file_count:1212
total size:1267811032 file_count:1212
```

## 将正在写入的volume迁移到hdd磁盘上
```
volume.tier.move -fromDiskType=ssd -toDiskType=hdd -fullPercent=4 -quietFor=0s -force
```

可以看到在持续的写入中，之前的volume 4已经迁移到hdd磁盘上，并且新的写入数据都写入到了volume 5中
```
DataCenter DefaultDataCenter hdd(volume:4/16 active:4 free:12 remote:0) ssd(volume:1/8 active:1 free:7 remote:0)
    Rack DefaultRack hdd(volume:4/16 active:4 free:12 remote:0) ssd(volume:1/8 active:1 free:7 remote:0)
      DataNode 172.23.156.57:8080 ssd(volume:1/8 active:1 free:7 remote:0)
        Disk ssd(volume:1/8 active:1 free:7 remote:0)
          volume id:5  size:2112977368  file_count:2015  version:3  modified_at_second:1732867434  disk_type:"ssd"
        Disk ssd total size:2112977368 file_count:2015
      DataNode 172.23.156.57:8080 total size:2112977368 file_count:2015
      DataNode 172.23.156.57:8081 hdd(volume:4/16 active:4 free:12 remote:0)
        Disk hdd(volume:4/16 active:4 free:12 remote:0)
          volume id:1  size:975280  file_count:4  version:3  compact_revision:1  modified_at_second:1732867423
          volume id:2  size:1139536  file_count:2  version:3  modified_at_second:1732867483
          volume id:3  size:841520  file_count:1  version:3  modified_at_second:1732867303
          volume id:4  size:8373262936  file_count:7985  version:3  modified_at_second:1732867396
        Disk hdd total size:8376219272 file_count:7992
      DataNode 172.23.156.57:8081 total size:8376219272 file_count:7992
    Rack DefaultRack total size:10489196640 file_count:10007
  DataCenter DefaultDataCenter total size:10489196640 file_count:10007
```

## 生成纠删码冗余
```
> ec.encode -volumeId=4 -force
markVolumeReadonly 4 on 172.23.156.57:8081 ...
generateEcShards  4 on 172.23.156.57:8081 ...
parallelCopyEcShardsFromSource 4 172.23.156.57:8081
allocate 4.[0 1 2 3 4 5 6 7 8 9 10 11 12 13] 172.23.156.57:8081 => 172.23.156.57:8081
mount 4.[0 1 2 3 4 5 6 7 8 9 10 11 12 13] on 172.23.156.57:8081
unmount 4.[] from 172.23.156.57:8081
delete 4.[] from 172.23.156.57:8081
delete volume 4 from 172.23.156.57:8081
```


```
Topology volumeSizeLimit:30000 MB ssd(volume:3/8 active:3 free:5 remote:0) hdd(volume:3/16 active:3 free:12 remote:0)
  DataCenter DefaultDataCenter hdd(volume:3/16 active:3 free:12 remote:0) ssd(volume:3/8 active:3 free:5 remote:0)
    Rack DefaultRack hdd(volume:3/16 active:3 free:12 remote:0) ssd(volume:3/8 active:3 free:5 remote:0)
      DataNode 172.23.156.57:8080 ssd(volume:3/8 active:3 free:5 remote:0)
        Disk ssd(volume:3/8 active:3 free:5 remote:0)
          volume id:7  size:8  version:3  modified_at_second:1732867727  disk_type:"ssd"
          volume id:6  size:8  version:3  modified_at_second:1732867727  disk_type:"ssd"
          volume id:5  size:2112977368  file_count:2015  version:3  modified_at_second:1732867434  disk_type:"ssd"
        Disk ssd total size:2112977384 file_count:2015
      DataNode 172.23.156.57:8080 total size:2112977384 file_count:2015
      DataNode 172.23.156.57:8081 hdd(volume:3/16 active:3 free:12 remote:0)
        Disk hdd(volume:3/16 active:3 free:12 remote:0)
          volume id:1  size:975280  file_count:4  version:3  compact_revision:1  modified_at_second:1732867423
          volume id:2  size:1139536  file_count:2  version:3  modified_at_second:1732867483
          volume id:3  size:841520  file_count:1  version:3  modified_at_second:1732867303
          ec volume id:4 collection: shards:[0 1 2 3 4 5 6 7 8 9 10 11 12 13]
        Disk hdd total size:2956336 file_count:7
      DataNode 172.23.156.57:8081 total size:2956336 file_count:7
    Rack DefaultRack total size:2115933720 file_count:2022
  DataCenter DefaultDataCenter total size:2115933720 file_count:2022
total size:2115933720 file_count:2022
```
seaweedfs默认的rs 10+4分片，将volume 4 rs编码之后写入到新的ec volume 4， 并且删除volume 4

之后用fuse profile 测试从ec volume中读数据
```
root@DESKTOP-2J0TFE0:/mnt/c/Users/tsukasa# python3 /mnt/f/source/buckyos/doc/dfs/fuse_profile.py 1 1048576 10000 /opt/se
aweed/ read
开始测试...
线程数: 1
单文件大小: 1048576 字节
总文件数: 10000
目标目录: /opt/seaweed/
测试模式: read

开始读取文件...
读取进度: 7168/10000 文件 已耗时: 185.2秒 平均速度: 38.71 MB/s
```
符合预期，在不需要rebuild的情况下，单hdd从ec volume单线程读出的速度与非ec volume基本一致
但是从windows资源管理器查看d和E盘的volume目录，发现所有的ec 分片都放置在相同磁盘上

## 分离两个hdd到两个data node上
```
Topology volumeSizeLimit:30000 MB hdd(volume:3/16 active:3 free:12 remote:0) ssd(volume:3/8 active:3 free:5 remote:0)
  DataCenter DefaultDataCenter ssd(volume:3/8 active:3 free:5 remote:0) hdd(volume:3/16 active:3 free:12 remote:0)
    Rack DefaultRack hdd(volume:3/16 active:3 free:12 remote:0) ssd(volume:3/8 active:3 free:5 remote:0)
      DataNode 172.23.156.57:8080 ssd(volume:3/8 active:3 free:5 remote:0)
        Disk ssd(volume:3/8 active:3 free:5 remote:0)
          volume id:6  size:8  version:3  modified_at_second:1732867727  disk_type:"ssd"
          volume id:5  size:2112977368  file_count:2015  version:3  modified_at_second:1732867434  disk_type:"ssd"
          volume id:7  size:8  version:3  modified_at_second:1732867727  disk_type:"ssd"
        Disk ssd total size:2112977384 file_count:2015
      DataNode 172.23.156.57:8080 total size:2112977384 file_count:2015
      DataNode 172.23.156.57:8081 hdd(volume:2/8 active:2 free:6 remote:0)
        Disk hdd(volume:2/8 active:2 free:6 remote:0)
          volume id:1  size:975280  file_count:4  version:3  compact_revision:1  modified_at_second:1732867423
          volume id:3  size:841520  file_count:1  version:3  modified_at_second:1732867303
        Disk hdd total size:1816800 file_count:5
      DataNode 172.23.156.57:8081 total size:1816800 file_count:5
      DataNode 172.23.156.57:8082 hdd(volume:1/8 active:1 free:6 remote:0)
        Disk hdd(volume:1/8 active:1 free:6 remote:0)
          volume id:2  size:1139536  file_count:2  version:3  modified_at_second:1732867483
          ec volume id:4 collection: shards:[0 1 2 3 4 5 6 7 8 9 10 11 12 13]
        Disk hdd total size:1139536 file_count:2
      DataNode 172.23.156.57:8082 total size:1139536 file_count:2
    Rack DefaultRack total size:2115933720 file_count:2022
  DataCenter DefaultDataCenter total size:2115933720 file_count:2022
total size:2115933720 file_count:2022
```
尝试ec.balance 并没有效果
同样的对刚刚写入的 volume 5移动到hdd并且ec之后, 得到了正确的结果，ec volume 5均匀的分布到两个hdd磁盘上
```
Topology volumeSizeLimit:30000 MB hdd(volume:3/16 active:3 free:11 remote:0) ssd(volume:2/8 active:2 free:6 remote:0)
  DataCenter DefaultDataCenter ssd(volume:2/8 active:2 free:6 remote:0) hdd(volume:3/16 active:3 free:11 remote:0)
    Rack DefaultRack hdd(volume:3/16 active:3 free:11 remote:0) ssd(volume:2/8 active:2 free:6 remote:0)
      DataNode 172.23.156.57:8080 ssd(volume:2/8 active:2 free:6 remote:0)
        Disk ssd(volume:2/8 active:2 free:6 remote:0)
          volume id:6  size:8  version:3  modified_at_second:1732867727  disk_type:"ssd"
          volume id:7  size:8  version:3  modified_at_second:1732867727  disk_type:"ssd"
        Disk ssd total size:16 file_count:0
      DataNode 172.23.156.57:8080 total size:16 file_count:0
      DataNode 172.23.156.57:8081 hdd(volume:2/8 active:2 free:6 remote:0)
        Disk hdd(volume:2/8 active:2 free:6 remote:0)
          volume id:1  size:975280  file_count:4  version:3  compact_revision:1  modified_at_second:1732867423
          volume id:3  size:841520  file_count:1  version:3  modified_at_second:1732867303
          ec volume id:5 collection: shards:[1 3 5 7 9 11 13]
        Disk hdd total size:1816800 file_count:5
      DataNode 172.23.156.57:8081 total size:1816800 file_count:5
      DataNode 172.23.156.57:8082 hdd(volume:1/8 active:1 free:5 remote:0)
        Disk hdd(volume:1/8 active:1 free:5 remote:0)
          volume id:2  size:1139536  file_count:2  version:3  modified_at_second:1732867483
          ec volume id:4 collection: shards:[0 1 2 3 4 5 6 7 8 9 10 11 12 13]
          ec volume id:5 collection: shards:[0 2 4 6 8 10 12]
        Disk hdd total size:1139536 file_count:2
      DataNode 172.23.156.57:8082 total size:1139536 file_count:2
    Rack DefaultRack total size:2956352 file_count:7
  DataCenter DefaultDataCenter total size:2956352 file_count:7
total size:2956352 file_count:7
```

## 结论
+ ssd作为写缓存，以固定策略（写了多少，空闲了多久）把ssd上的热数据移动到hdd，并且纠删编码在流程上是ok的
+ 每一块硬盘需要单独分配一个port挂在单独的volume server上，才能保证纠删片可以均匀的分布在不同的硬盘上
+ 似乎存在一个bug，生成纠删片时如果只有一个volume server，之后在balance也无法均匀分布纠删片（正确的初始配置可以规避这个问题）
+ 可以通过 volume.tier.move -toReplication参数改变移动到hdd的时候执行写复制，但是在删除ssd上的volume之后， hdd上仍然只有一个副本，需要一次额外的 volume.fix.replication才能产生第二个副本， 或者在执行ec之前都是只有hdd上的单一副本，这个时候的可靠性是有问题的；


# 把ssd当作读缓存
没有方案


# 扩容
## 双ssd作为raid 1写缓存
把c盘的ssd挂到单独的volume server加进来
```
Topology volumeSizeLimit:30000 MB hdd(volume:3/16 active:3 free:11 remote:0) ssd(volume:4/16 active:4 free:12 remote:0)
  DataCenter DefaultDataCenter ssd(volume:4/16 active:4 free:12 remote:0) hdd(volume:3/16 active:3 free:11 remote:0)
    Rack DefaultRack hdd(volume:3/16 active:3 free:11 remote:0) ssd(volume:4/16 active:4 free:12 remote:0)
      DataNode 172.23.156.57:8080 ssd(volume:4/8 active:4 free:4 remote:0)
        Disk ssd(volume:4/8 active:4 free:4 remote:0)
          volume id:6  size:8  version:3  compact_revision:1  modified_at_second:1732871056  disk_type:"ssd"
          volume id:8  size:8  version:3  compact_revision:1  modified_at_second:1732871066  disk_type:"ssd"
          volume id:7  size:8  version:3  compact_revision:1  modified_at_second:1732871076  disk_type:"ssd"
          volume id:9  size:8  version:3  compact_revision:1  modified_at_second:1732871086  disk_type:"ssd"
        Disk ssd total size:32 file_count:0
      DataNode 172.23.156.57:8080 total size:32 file_count:0
      DataNode 172.23.156.57:8081 hdd(volume:2/8 active:2 free:6 remote:0)
        Disk hdd(volume:2/8 active:2 free:6 remote:0)
          volume id:1  size:3779104  file_count:6  version:3  compact_revision:1  modified_at_second:1732870543
          volume id:3  size:3176240  file_count:3  version:3  modified_at_second:1732870963
          ec volume id:5 collection: shards:[1 3 5 7 9 11 13]
        Disk hdd total size:6955344 file_count:9
      DataNode 172.23.156.57:8081 total size:6955344 file_count:9
      DataNode 172.23.156.57:8082 hdd(volume:1/8 active:1 free:5 remote:0)
        Disk hdd(volume:1/8 active:1 free:5 remote:0)
          volume id:2  size:2367232  file_count:4  version:3  modified_at_second:1732870723
          ec volume id:4 collection: shards:[0 1 2 3 4 5 6 7 8 9 10 11 12 13]
          ec volume id:5 collection: shards:[0 2 4 6 8 10 12]
        Disk hdd total size:2367232 file_count:4
      DataNode 172.23.156.57:8082 total size:2367232 file_count:4
      DataNode 172.23.156.57:8083 ssd(volume:0/8 active:0 free:8 remote:0)
        Disk ssd(volume:0/8 active:0 free:8 remote:0)
        Disk ssd total size:0 file_count:0
      DataNode 172.23.156.57:8083 total size:0 file_count:0
    Rack DefaultRack total size:9322608 file_count:13
  DataCenter DefaultDataCenter total size:9322608 file_count:13
total size:9322608 file_count:13
```

改变filer 的写复制模式为 001
```
> fs.configure -locationPrefix=/ -replication=001 -apply
{
  "version":  0,
  "locations":  [
    {
      "locationPrefix":  "/",
      "collection":  "",
      "replication":  "001",
      "ttl":  "",
      "diskType":  "ssd",
      "fsync":  false,
      "volumeGrowthCount":  1,
      "readOnly":  false,
      "dataCenter":  "",
      "rack":  "",
      "dataNode":  "172.23.156.57:8080",
      "maxFileNameLength":  0,
      "disableChunkDeletion":  false,
      "worm":  false
    }
  ]
}
```

还是之前的10K个1M文件写入，写入速度和单ssd差不多，虽然写复制但是并发写，符合预期;
可以看到新增的volume 11在两块ssd上各有一个副本

```
opology volumeSizeLimit:30000 MB hdd(volume:5/16 active:5 free:9 remote:0) ssd(volume:6/16 active:6 free:10 remote:0)
  DataCenter DefaultDataCenter hdd(volume:5/16 active:5 free:9 remote:0) ssd(volume:6/16 active:6 free:10 remote:0)
    Rack DefaultRack hdd(volume:5/16 active:5 free:9 remote:0) ssd(volume:6/16 active:6 free:10 remote:0)
      DataNode 172.23.156.57:8080 ssd(volume:5/8 active:5 free:3 remote:0)
        Disk ssd(volume:5/8 active:5 free:3 remote:0)
          volume id:6  size:8  version:3  compact_revision:1  modified_at_second:1732871056  disk_type:"ssd"
          volume id:8  size:8  version:3  compact_revision:1  modified_at_second:1732871066  disk_type:"ssd"
          volume id:7  size:8  version:3  compact_revision:1  modified_at_second:1732871076  disk_type:"ssd"
          volume id:9  size:8  version:3  compact_revision:1  modified_at_second:1732871086  disk_type:"ssd"
          volume id:11  size:7004808328  file_count:6680  replica_placement:1  version:3  modified_at_second:1732871722  disk_type:"ssd"
        Disk ssd total size:7004808360 file_count:6680
      DataNode 172.23.156.57:8080 total size:7004808360 file_count:6680
      DataNode 172.23.156.57:8081 hdd(volume:3/8 active:3 free:5 remote:0)
        Disk hdd(volume:3/8 active:3 free:5 remote:0)
          volume id:1  size:3779104  file_count:6  version:3  compact_revision:1  modified_at_second:1732870543
          volume id:3  size:3176240  file_count:3  version:3  modified_at_second:1732870963
          volume id:10  size:1464336  file_count:3  replica_placement:1  version:3  modified_at_second:1732871683
          ec volume id:5 collection: shards:[1 3 5 7 9 11 13]
        Disk hdd total size:8419680 file_count:12
      DataNode 172.23.156.57:8081 total size:8419680 file_count:12
      DataNode 172.23.156.57:8082 hdd(volume:2/8 active:2 free:4 remote:0)
        Disk hdd(volume:2/8 active:2 free:4 remote:0)
          volume id:2  size:2367232  file_count:4  version:3  modified_at_second:1732870723
          volume id:10  size:1464336  file_count:3  replica_placement:1  version:3  modified_at_second:1732871683
          ec volume id:4 collection: shards:[0 1 2 3 4 5 6 7 8 9 10 11 12 13]
          ec volume id:5 collection: shards:[0 2 4 6 8 10 12]
        Disk hdd total size:3831568 file_count:7
      DataNode 172.23.156.57:8082 total size:3831568 file_count:7
      DataNode 172.23.156.57:8083 ssd(volume:1/8 active:1 free:7 remote:0)
        Disk ssd(volume:1/8 active:1 free:7 remote:0)
          volume id:11  size:7005856952  file_count:6681  replica_placement:1  version:3  modified_at_second:1732871722  disk_type:"ssd"
        Disk ssd total size:7005856952 file_count:6681
      DataNode 172.23.156.57:8083 total size:7005856952 file_count:6681
    Rack DefaultRack total size:14022916560 file_count:13380
  DataCenter DefaultDataCenter total size:14022916560 file_count:13380
total size:14022916560 file_count:13380
```
之后还是可以离线移动volume 11到hdd上，完成之后volume 11的两个ssd副本变成hdd上的一个副本

## 添加hdd后扩容
把h盘挂到单独的volume server上加进来
```
Topology volumeSizeLimit:30000 MB ssd(volume:10/16 active:10 free:6 remote:0) hdd(volume:9/24 active:9 free:11 remote:0)
  DataCenter DefaultDataCenter hdd(volume:9/24 active:9 free:11 remote:0) ssd(volume:10/16 active:10 free:6 remote:0)
    Rack DefaultRack hdd(volume:9/24 active:9 free:11 remote:0) ssd(volume:10/16 active:10 free:6 remote:0)
      DataNode 172.23.156.57:8080 ssd(volume:7/8 active:7 free:1 remote:0)
        Disk ssd(volume:7/8 active:7 free:1 remote:0)
          volume id:15  size:8  replica_placement:1  version:3  modified_at_second:1732873048  disk_type:"ssd"
          volume id:14  size:8  replica_placement:1  version:3  modified_at_second:1732872199  disk_type:"ssd"
          volume id:16  size:8  replica_placement:1  version:3  modified_at_second:1732873048  disk_type:"ssd"
          volume id:6  size:8  version:3  compact_revision:1  modified_at_second:1732871056  disk_type:"ssd"
          volume id:8  size:8  version:3  compact_revision:1  modified_at_second:1732871066  disk_type:"ssd"
          volume id:7  size:8  version:3  compact_revision:1  modified_at_second:1732871076  disk_type:"ssd"
          volume id:9  size:8  version:3  compact_revision:1  modified_at_second:1732871086  disk_type:"ssd"
        Disk ssd total size:56 file_count:0
      DataNode 172.23.156.57:8080 total size:56 file_count:0
      DataNode 172.23.156.57:8081 hdd(volume:5/8 active:5 free:2 remote:0)
        Disk hdd(volume:5/8 active:5 free:2 remote:0)
          volume id:12  size:1733944  file_count:2  replica_placement:1  version:3  modified_at_second:1732872523
          volume id:13  size:8  replica_placement:1  version:3  modified_at_second:1732872198
          volume id:1  size:3779104  file_count:6  version:3  compact_revision:1  modified_at_second:1732870543
          volume id:3  size:3176240  file_count:3  version:3  modified_at_second:1732870963
          volume id:10  size:2942848  file_count:6  replica_placement:1  version:3  modified_at_second:1732871863
          ec volume id:5 collection: shards:[1 3 5 7 9 11 13]
          ec volume id:11 collection: shards:[1 3 5 7 9 11 13]
        Disk hdd total size:11632144 file_count:17
      DataNode 172.23.156.57:8081 total size:11632144 file_count:17
      DataNode 172.23.156.57:8082 hdd(volume:4/8 active:4 free:2 remote:0)
        Disk hdd(volume:4/8 active:4 free:2 remote:0)
          volume id:10  size:2942848  file_count:6  replica_placement:1  version:3  modified_at_second:1732871863
          volume id:12  size:1733944  file_count:2  replica_placement:1  version:3  modified_at_second:1732872523
          volume id:13  size:8  replica_placement:1  version:3  modified_at_second:1732872199
          volume id:2  size:2367232  file_count:4  version:3  modified_at_second:1732870723
          ec volume id:4 collection: shards:[0 1 2 3 4 5 6 7 8 9 10 11 12 13]
          ec volume id:5 collection: shards:[0 2 4 6 8 10 12]
          ec volume id:11 collection: shards:[0 2 4 6 8 10 12]
        Disk hdd total size:7044032 file_count:12
      DataNode 172.23.156.57:8082 total size:7044032 file_count:12
      DataNode 172.23.156.57:8083 ssd(volume:3/8 active:3 free:5 remote:0)
        Disk ssd(volume:3/8 active:3 free:5 remote:0)
          volume id:15  size:8  replica_placement:1  version:3  modified_at_second:1732873048  disk_type:"ssd"
          volume id:14  size:8  replica_placement:1  version:3  modified_at_second:1732872199  disk_type:"ssd"
          volume id:16  size:8  replica_placement:1  version:3  modified_at_second:1732873048  disk_type:"ssd"
        Disk ssd total size:24 file_count:0
      DataNode 172.23.156.57:8083 total size:24 file_count:0
      DataNode 172.23.156.57:8084 hdd(volume:0/8 active:0 free:8 remote:0)
        Disk hdd(volume:0/8 active:0 free:8 remote:0)
        Disk hdd total size:0 file_count:0
      DataNode 172.23.156.57:8084 total size:0 file_count:0
    Rack DefaultRack total size:18676256 file_count:29
  DataCenter DefaultDataCenter total size:18676256 file_count:29
total size:18676256 file_count:29
```
执行ec.balance 之后
```
 DataNode 172.23.156.57:8081 hdd(volume:5/8 active:5 free:2 remote:0)
        Disk hdd(volume:5/8 active:5 free:2 remote:0)
          volume id:3  size:3176240  file_count:3  version:3  modified_at_second:1732870963
          volume id:10  size:2942848  file_count:6  replica_placement:1  version:3  modified_at_second:1732871863
          volume id:12  size:1733944  file_count:2  replica_placement:1  version:3  modified_at_second:1732872523
          volume id:13  size:8  replica_placement:1  version:3  modified_at_second:1732872198
          volume id:1  size:3779104  file_count:6  version:3  compact_revision:1  modified_at_second:1732870543
          ec volume id:5 collection: shards:[5 7 9 11 13]
          ec volume id:11 collection: shards:[5 7 9 11 13]
          ec volume id:4 collection: shards:[5 6 7 8]
        Disk hdd total size:11632144 file_count:17
      DataNode 172.23.156.57:8081 total size:11632144 file_count:17
      DataNode 172.23.156.57:8082 hdd(volume:4/8 active:4 free:3 remote:0)
        Disk hdd(volume:4/8 active:4 free:3 remote:0)
          volume id:2  size:2367232  file_count:4  version:3  modified_at_second:1732870723
          volume id:10  size:2942848  file_count:6  replica_placement:1  version:3  modified_at_second:1732871863
          volume id:12  size:1733944  file_count:2  replica_placement:1  version:3  modified_at_second:1732872523
          volume id:13  size:8  replica_placement:1  version:3  modified_at_second:1732872199
          ec volume id:4 collection: shards:[9 10 11 12 13]
          ec volume id:5 collection: shards:[4 6 8 10 12]
          ec volume id:11 collection: shards:[4 6 8 10 12]
        Disk hdd total size:7044032 file_count:12
      DataNode 172.23.156.57:8082 total size:7044032 file_count:12
      DataNode 172.23.156.57:8083 ssd(volume:3/8 active:3 free:5 remote:0)
        Disk ssd(volume:3/8 active:3 free:5 remote:0)
          volume id:15  size:8  replica_placement:1  version:3  modified_at_second:1732873048  disk_type:"ssd"
          volume id:14  size:8  replica_placement:1  version:3  modified_at_second:1732872199  disk_type:"ssd"
          volume id:16  size:8  replica_placement:1  version:3  modified_at_second:1732873048  disk_type:"ssd"
        Disk ssd total size:24 file_count:0
      DataNode 172.23.156.57:8083 total size:24 file_count:0
      DataNode 172.23.156.57:8084 hdd(volume:0/8 active:0 free:7 remote:0)
        Disk hdd(volume:0/8 active:0 free:7 remote:0)
          ec volume id:5 collection: shards:[0 1 2 3]
          ec volume id:11 collection: shards:[0 1 2 3]
          ec volume id:4 collection: shards:[0 1 2 3 4]
```
ec 分片均匀的分布了



