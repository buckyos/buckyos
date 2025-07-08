# process_chain_cmd

## 语句结构

- 条件执行
cmd1 parm1 parm2 && cmd2 param3 //当cmd1执行成功，执行cmd2
cmd1 param1 param2 || cmd2 param3 //当cmd1执行失败，执行cmd2
cmd1 && cmd2 || cmd3 //如果cmd1成功则支持cmd2 否则支持 cmd3

- 子命令
cmd1 parm1 $(cmd2 parm2) // cmd2 param2的返回值为cmd1的第二个参数

## 匹配命令

### match 变量 表达式
这是最常用的匹配语句
简单匹配，表达为传统的 文件名表达式 *.abc
*的匹配规则：
    不区分大小写
    为域名（文件名有效字符）

使用案例
域名匹配  
- *.web3.buckyos.ai
- home.*.buckyos.ai

IP地址匹配
- 192.168.0.*

### eq 变量1 变量2
完全相同

### match_reg 变量 正则表达式
标准正则表达式，匹配成功后会写入环境变量 MATCH[0],MATCH[1].. 

### range 变量 最小值 最大值
判断变量是否处于区间范围内

## 流程控制
### policy key value
设置本block的运行配置,比如当命令出错的标准处理（默认是跳过）

### exec $sub_id
执行一个sub并得到返回值
exec 下面一行的命令会继续执行

### goto $chain_id
跳转到另一个chain,
goto 下面一行的命令不会继续执行

### return 变量
从当前sub成功返回，返回值为变量x

## error
从当前sub错误返回

### exit 返回值
终止当前process_chain 

### drop 
相当于 exit "drop"

### accept
相当于 exit "accept"

## 字符串操作 (最常用的嵌入命令)

### rewrite 变量名 匹配语句 参数1 参数2
最常用的命令，匹配成功后，对变量进行修改
```
rewrite $REQ.url /kapi/my-service/* /kapi/*
```

### rewrite_reg 变量名 匹配语句 参数1 参数2
用正则表达式进行匹配替换
```
```

### replace 变量名 匹配语句 新值
比如将 字符串中的 的io替换成ai
replace $REQ.host io ai

### append 字符串1 字符串2
将字符串2拼接到字符串1后面

### slice 字符串1 0:5
返回字符串的 一个部分

### strlen 字符串
发挥字符串的长度

### startwith 变量 字符串
变量是否以字符串开头

### endwith 变量 字符串
变量是否以字符串结尾


## 集合管理

### match_include 变量1 集合1
判断变量1是否在集合1中
```
match_include REQ_target_ip
```
### set_craete setid
创建set,如果setid已经存在，则返回失败

### map_create mapid
创建map,如果mapid已经存在，则返回失败

### set_add setid 变量1
将变量1添加到setid代表的set中，如果该变量在Set中不存在，则返回成功

### map_add mapid key value
将(key-value) 增加到mapid

### set_remove setid value
从指定set中删除一个item

### map_remove mapid key
从指定map中删除一个item


## 变量管理

## export 变量=值
要求将变量设置为指定值

## delete 变量
要求删除变量

## 调试支持
echo "xxxx" debug

## 一些典型的标准环境变量
标准环境变量都是大写的,我们希望找到一个格式，可以更优雅一些的解决bash不能用类似REQ.url的方式来表达一个变量的问题

$PROTOCOL

${REQ_schema}
$REQ_host
$REQ_url
$REQ_from_ip
$REQ_from_port
$REQ_target_ip
$REQ_target_port
$REQ_seq

$REQ_Content_Length

$IP_DB IP数据库,可以根据IP返回一组tags
$HOST_DB 域名数据库，可以根据域名返回一组tags