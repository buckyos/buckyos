# rbac 的基本流程

使用: 
```rust
BuckyOSRuntime::enforce(&self,req:&RPCRequest, action: &str,resource_path: &str) -> Result<(String,String)> 
``` 

enforce流程
- 带cache的读取system/rbac/model和system/rbac/policy
- 如果发生改变，则修改全局的SYS_ENFORCE对象
- 调用SYS_ENFORCE.enforce(userid,appid,res_path,op_name) 得到结果

因为cache的存在，所以system/rbac/policy，最长10秒系统里的所有service才会看到
TODO：通过system_config的watch机制，来主动触发cache失效，提高权限配置修改后系统应用的速度。

## boot流程
- 初始化的时候，系统会写入system/rbac/base_policy和system/rbac/model
- 如果boot.template.toml没有配置（通常不配置），则使用rbac/lib.rs 里，固化在代码里的默认配置

## 构造system/rbac/policy
- 调度器负责构造 policy
- 考虑到权限变化无法完全与 应用/服务/用户/node的增删 相关，因此不会主动构造，而需要显示的调用调度器接口触发构造

## 理解系统的基本权限配置
（参考系统的权限配置文档)

## sudo 需求
普通用户，在处理自己的数据的时候，也需要通过分离普通权限和高级权限来保护一些关键数据

当需要进行高权限的操作时，需要一个临时的sudo token(通常有效期很短)，该token通常需要用另一种验证方式产生
比如需要用户再输入一次密码，或则强制需要用户私钥匙签名

### sudo的具体实现

1. 权限文件的修改

owner用户天然就是走的SUDO流程
创建super_user，super_admin用户组
任何用户在创建的时候，会同步创建另一个super_xxxx的用户（反过来，系统里不允许用户名以super_开头），加入super_user / super_admin组
分配4个用户组的权限（注意测试）

2. verify_hub修改

是否要支持在OOD上保存用户的私钥？
当申请super系列的token时，处理特殊逻辑

- super_xxx 的 token申请, 给的token时间很短
- super_admin, 只接受件jwt方式的申请,要求构造签名

3. 界面修改
创建用户的入口，增加合法用户名检查
当用户申请管理员权限的时候，根据配置弹出 要求构造JWT或密码的界面
sudo的login界面要有不同

