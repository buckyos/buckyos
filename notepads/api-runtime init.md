
## API Runtime Init review


P1: “每进程只允许一个 runtime” 现在不是一个原子契约，而只是调用约定。init_buckyos_api_runtime() 只检查全局 cell 当前是否为空，然后返回一个未注册的 BuckyOSRuntime，lib.rs (line 149)；真正注册发生在后续单独的 set_buckyos_api_runtime()，而这个 setter 又静默吞掉失败，lib.rs (line 70)。这意味着两个调用方可以同时成功 init + login，最后只有一个真正注册，但两个都可能已经启动了后台 keep-alive，runtime.rs (line 819)。初始化语义因此依赖调用顺序和时序，而不是 API 本身。

P1: CURRENT_DEVICE_CONFIG 把初始化变成了“依赖历史副作用”的过程。fill_by_load_config() 只有在全局 CURRENT_DEVICE_CONFIG 为空时，才会把 device_config 和 zone_id 写入当前 runtime，runtime.rs (line 420)；如果这个全局值之前已经被别的 runtime 设过，当前 runtime 会跳过这段逻辑。随后 fill_by_env_var() 只会把 device_config 从全局回填回来，不会回填 zone_id，runtime.rs (line 220)。结果就是：同样的配置，在“第一次初始化”和“进程里第二次初始化”得到的 runtime 完整度不同，后者甚至可能直到 login() 才因为 zone_id 无效失败。

P1: init() 成功并不代表 runtime 已经达到统一的“可登录”状态，但类型系统完全没有表达这个差异。init_buckyos_api_runtime() 对 AppService 会跳过配置文件 bootstrap，只依赖输入参数和环境变量，lib.rs (line 230) runtime.rs (line 293)；而 KernelService/AppClient 则会加载 node identity、private key、owner config 等较完整状态，runtime.rs (line 367)。同一个 BuckyOSRuntime 类型，在不同 runtime type 下其实处于不同“相位”，但调用方拿不到任何相位信息，只能靠经验判断哪些字段这时可用。

P2: 初始化阶段已经把 user_id 混成了三个不同语义。加载 node identity 时，user_id 被写成设备名，runtime.rs (line 442)；AppService 又在 env 路径里把它改回 owner，runtime.rs (line 226)；AppClient 如果发现本地 user_private_key.pem，又会把 user_id 强行改成 "root"，runtime.rs (line 483)。这让 user_id 有时代表登录用户，有时代表设备身份，有时代表签名主体，后续很多调用方又把它直接当“当前用户”使用，例如 main.rs (line 476)。

P2: 初始化语义已经从“创建 singleton”演进成“构造一个未注册 runtime，再由调用方决定何时登录和注册”，但旧心智模型还残留在文档和接口命名里。当前代码路径是 init -> login -> set_global，例如 main.rs (line 7439) 和 main.rs (line 803)；但旧文档仍把 init_buckyos_api_runtime() 描述成创建 runtime 单件，buckyos-api-runtime.md (line 1)。这会持续制造误用，因为 API 名字和实际相位模型已经不一致。
Open Questions

AppClient 发现本地开发私钥后切到 "root"，这是刻意的“开发者即 root”语义，还是历史兼容留下来的折中？如果是前者，建议把“逻辑用户”和“签名主体”拆成两个字段，而不是复用 user_id。
“一个进程只能有一个 runtime” 到底是硬约束，还是目前实现上的便利假设？如果是硬约束，init 和 set 应该合并成一个原子流程；如果不是，就不该让 init() 依赖全局 cell 状态。
AppService 的初始化是否真的允许在没有 zone_id/device_config 的情况下成功返回？如果允许，那应该有显式 phase；如果不允许，校验应前移到 init()。
Summary
我对这块的判断是：最大的问题不是“逻辑复杂”，而是 runtime 已经从“对象构造”演变成“多阶段、带全局副作用的生命周期机”，但接口仍然假装它只是一个普通初始化函数。现在最值得收敛的是 3 件事：原子化注册语义、去掉 CURRENT_DEVICE_CONFIG 对初始化结果的隐式劫持、把 identity/phase 显式建模出来。

这次是源码 review，没有做并发复现；现有测试里我只看到 happy-path 的 AppService env bootstrap 覆盖，lib.rs (line 316)，没有覆盖 singleton/register/CURRENT_DEVICE_CONFIG 这些真正容易漂移的语义点。