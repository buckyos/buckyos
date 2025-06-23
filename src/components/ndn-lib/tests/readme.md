# 本目录是对ndn-lib的测试用例

# 先简单描述一下我对本模块结构和一些关键概念的理解

* 数据分为两大类：

    1. Chunk: 非结构化的数据块
    2. Object: 结构化数据，格式为任意的`json`字符串
        * 有几个内置Object: File/Dir等

* ndn-lib主要包含几个接口组件：

    1. NamedDataMgr: 负责本地数据存取
    2. NdnClient: 负责跨NamedDataMgr、跨设备、跨Zone数据交换
    3. 可以直接通过`http`协议访问
    
* 数据组织:

    1. 所有数据(包括`Chunk`和任意`Object`)都能按照特定的规则制定一个`ID`（通常是某种`HASH`）为其命名；具体命名规则可以由用户自己制定，也可以采用系统默认的规则。
    2. 直接存入NamedDataMgr，以`ID`检索
    3. 系统有一个树形结构，可以将任意数据挂载到任意叶子节点上，这个叶子节点路径即是该数据的`NdnPath`；一个节点只能挂载一个对象。
    4. `Object`是一个`json`格式，它也是一个树形结构，其中各子节点（子对象）也有相应的路径(`inner-path`)，要想单独检索子对象可以在检索根对象(`root-object`)的同时加入`inner-path`参数

# 用例设计

* 通过上述对模块结构的整理，测试用例按以下几个维度进行设计：

    1. 数据类型: Chunk, Object, File
    2. 存取接口: NamedDataMgr, NdnClient, http
    3. 检索方式: ID, NdnPath, inner-path
    4. 设备拓扑：同`NamedDataMgr`，2个`NamedDataMgr`，同`Zone`不同设备，跨`Zone`设备

    *** http: 基本和NdnClient的实现雷同，复制代码意义不大，考虑后面有不同实现的SDK后(如Python, JS等)，直接使用不同实现的SDK代替 ***

    *** 同`Zone`不同设备，目前没有实现这种功能，暂时不添加测试 ***

* 测试环境

    * 不涉及到`zone`的用例，直接本地测试即可：`cargo test`
    * 涉及到`zone`的用例，在执行`cargo test`前先启动标准开发环境，其中包括几个`zone`：
        
        1. 本地开发环境：test.buckyos.io
        2. bob.web3.buckyos.io

# 各用例按照拓扑结构分别实现在不同的文件

1. local_signal_ndn_mgr_chunk.rs: 本地只有一个`NamedDataMgr`的`chunk`
2. local_signal_ndn_mgr_obj.rs: 本地只有一个`NamedDataMgr`的`Object`
3. local_signal_ndn_mgr_file_api.rs: 本地只有一个`NamedDataMgr`的`File`
4. local_2_ndn_mgr_chunk.rs: 本地2个`NamedDataMgr`的`chunk`
5. local_2_ndn_mgr_obj.rs: 本地2个`NamedDataMgr`的`Object`
6. local_2_ndn_mgr_file_api.rs: 本地2个`NamedDataMgr`的`File`
7. ndn_2_zone_test_chunk.rs: 2个`zone`的`chunk`
8. ndn_2_zone_test_obj.rs: 2个`zone`的`Object`
9. ndn_2_zone_file_api.rs: 2个`zone`的`file`


        
