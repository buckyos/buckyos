## Test测试服务器
标准http服务

充当github action和线下multipass测试，package push服务之间的桥梁

保存版本的状态信息：测试结果、发布结果

后续可以增加更多基于版本号的信息

### 功能
- 保存github action编译结果的url
- 可以在必要的时候将编译结果保存在本地
- 供测试服务器查询
- 测试服务器上报测试结果
- 供打包服务器查询
- 打包服务器上报发布结果

### 流程
1. github action触发，编译6个平台，并upload artifaces
2. action将upload的url上报到服务器
3. 测试服务器定期查询未测试的版本
4. 通过url下载未测试的linux x86版本，并开始测试
5. 测试结束后，上报测试结果
6. 打包服务器定期查询是否有通过测试，且未打包上传的版本
7. 打包服务器下载对应版本，打包并发布
8. 发布后上传发布结果

### 安全
只有预先添加的用户，才能调用POST接口上报信息。信息使用椭圆曲线签名

POST通用结构：
```json
{
    "content": "",
    "user": "",
    "sig": ""
}
```

以下接口说明中，所有的POST接口都只标识content

### 接口

#### POST /version/url
上报编译信息
`content`:
```json
{
    "version": "",
    "arch": "",
    "os": "",
    "url": ""
}
```

#### GET /version?page=&size=&arch=&os=&notest=&nopub=
查询版本对应的包信息。所有参数均为可选
- page: 要查询的起始页，从1开始，分页用
- size: 要查询的页大小，分页用，不传或传入0则表示不分页。
- arch：符合的cpu体系。可以多次传入多个值，
- os：符合的操作系统。可以多次传入多个值，
- notest: 传入true或false。是否只返回未测试的版本。默认值为false
- nopub: 传入true或false。是否只返回未发布的版本。默认值为false

返回:
```json
{
    "page": 0,
    "size": 10,
    "items": [
        {
            "version": "",
            "os": "",
            "arch": "",
            "tested": 1,      //1表示通过，-1表示未通过，0表示未执行
            "published": 1
        }
    ]
}
```

#### GET /version/total
查询版本总数，分页用
返回：
```json
{
    "total": 10
}

#### POST /version/test
上报测试结果信息
`content`:
```json
{
    "result": 1
}
```

#### POST /version/publish
上报打包结果信息
`content`:
```json
{
    "result": 1
}
```