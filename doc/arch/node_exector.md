# node_exector 需求

node_exector 的目的是执行scheduler下发的thunk object(Function Instance)
这是node_daemon中的一个子模块，基本结构可以参考app_loader.rs

## 基本逻辑方式

- node_daemon得到本机的thunkobject
- 根据function_type决定是否要执行
- 启动node_runner执行thunkojbect
- 一个ThunkObject只能存在一个运行实例

## node_exector运行不同类型的ThunkObject:

两大类：
1. 直接运行（脚本/pkg),主要是构造好参数
2. 用容器运行，配置好容器启动，配置好运行参数

- OPTask : 运维脚本，执行后会影响NodeState，一般是bash脚本。肯定不会在容器里运行
- ExecPkg : 启动一个Pkg进程，然后把ThunkObject当参数传递给该Pkg。根据pkg的类型，决定是否在容器里运行
- Script ： 运行脚本,默认在aios的node_executor容器里运行(比如调用Py)
- Operator 算子，暂时不支持（为跑本地FineTune/训练预留）



## Task状态管理

负责管理 thunkobject -> executor(pid),防止重复启动,并会做必要的超时管理
负责管理thunkobject的取消
负责设置task的 Running状态：成功 / 失败 / 超时
不管理task.data,这个是具体的Func实现里管理的，如果Func实现里不管，那么data里只有标准的metadata

## Func执行协议
THIS_THUNK = thunk_json_str
func_exe > executor_result.json 

executor_result.json 的内容ThunkExecutionResult的json






