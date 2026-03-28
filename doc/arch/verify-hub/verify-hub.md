# 理解buckyos的身份体系

## 自验证流程

objid->namedobject , hash验证,objid里是内容相关的，和授权部分（签名段）无关
did->did_document ，授权验证（提供了可验证的身份体系）


## 常见的业务验证需要

一个业务在处理请求的时候，需要得到请求的下面关键要素
- action
- target resource path list
- operation userid
- operation appid

OOD/设备启动的时候，首先就是要知道自己所在的zone情况
    通过阅读设备配置文件，可以知晓：自己所在的zoneid,设备的owner,设备本身的did/device_info,设备自己的私钥

启动的时候，首先需要得到一个可信的ZoneConfig,然后根据这个ZoneConfig决定自己的下一步的动作
    得到可信zoneconfig的方法
        1. 验证zoneid->did_document(zoneconfig)，这个过程是did有限的
        2. 验证zoneconfig里的owner和设备的owner是相同的 （这意味着当zone的owner改变后，zone内的所有设备必须重新激活，防止zoneid配置被攻击后的潜在的隐私问题）。反过来，如果私钥丢失，只需要修改了zoneid的配置，那么所有的设备都会处于启动失败的状态。
        理解这个双向验证机制可以进一步理解系统对风险的管理。
        
        黑客攻击得到了owner密钥：用户用zone管理密钥（这甚至可以是传统的中心化身份）可以让zone不可用
        黑客攻击得到了zone管理密钥：不会修改

        


