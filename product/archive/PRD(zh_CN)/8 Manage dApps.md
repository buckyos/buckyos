# dApp管理

## dApp的安装

产品上，用户默认通过dApp Store安装dAapp, dApp Store是一个内置的应用，可以搜索并安装dApp。

### 通过上传dApp pkg安装

dApp Store是一个应用，并不是BuckyOS的必须的一部分。BuckyOS默认支持的是通过上传dApp pkg安装dApp。
有两种方法上传dApp pkg：

1. 用户通过添加一个指向dApp pkg的URL来上传dApp pkg
2. 用户通过上传一个dApp pkg文件来上传dApp pkg


### 增加新的dApp源

BuckyOS底层通过pkg mgr来管理所有的pkg, 这是一个完善的机制，在开发模式下用户可以不依赖任何其它设施实现pkg的安装、搜索、和源添加功能。

但在产品上，增加dApp源是dApp Store的功能。dApp Store增加源后，用户有机会搜索到更多的dApp。

### dApp的认证

dApp Store的本质是一个基于源的dApp-DB. 起的是收录/和编辑的作用。基于BuckyOS的 Zero Trust原则，对dApp的认证和收录是分离的。用户可以基于自己的信任，自由的添加自己可信的认证组织，并从已认证的dApp中选择安装。

### 付费应用

为了生态的健康发展，BuckyOS虽然是开源的，但也支持付费应用。
考虑到传统的付费渠道涉及到法币支付，必然需要实体，会影响BuckyOS的Zero trust原则，我们的付费应用是基于数字货币的。用户可以通过数字货币支付来购买dApp。

付费的认证通常在dApp 发布者的源上，用户每次付费后，才能下载dApp pkg.该pkg是加密的，只有在用户的设备上才能解密。

## dApp 安装引导

1. 等待pkg保存到BuckyOS
2. 进入安装流程，首先就是展示dApp的权限要求，用户可以选择同意授权
3. 执行安装
4. 提示安装成功,进入首次运行向导（由应用提供），一般是进行一些配置，这些过程中可能会需要用户同意授权。
5. 安装成功


## 已安装dApp的管理

### 删除

### 权限管理

