# Rust的一些基础规范

## Result类型

根本上是几个依赖关系穿透下来的

- buckyos-kit里的工具函数，特别是name-client/name-lib 库，有传导一些类型下来。应该分开看
  - 无状态的纯函数，可以提供一组基础的错误
- 从系统的角度来看，由于大量的系统调用是kRPC,所以RPCError是一个最常见的Reulst
- 当使用cyfs-ndn的时候，会遇到大量的NdnResult,里面的错误通常是和cyfs://协议相关，NamedObject的状态相关
- 


### 思考
错误类型的根本还是让上层有足够的方式可以处理，而不是简单的当异常。从常见的系统愿意处理的异常来看
- NotFound
- AlreadyExist
- Access Deny
- ParserError (错误，一般是用户输入的文件格式错了)
- DataError （异常，系统自己构造的数据坏了）
- ReasonError (异常，但是提供了逐级传导的异常信息)

多使用anyhow?

## Log使用


## 锁的控制

### 1）async fn foo(&Self ... )

这种一般内部所有的变量都已经适当的Arc<Mutex<T>>了
对使用者友好，一路clone()就好了，但死锁风险最大

### 2）async fn foo(Arc<Mutex<T>> ...) 
在接口里明确的说明了同步模式

### 3）async fn foo(&mut self)
把锁控制交给外围,这一类对象通常不会在锁控制上有死锁风险（标准Object/Struct)

结论：实现1）的对象必须叫xxxContext, 不允许使用方式2）来模拟成员函数，通常是global function

对，Rust 的编译器能保证内存安全，但死锁不在它的能力范围内。锁的顺序是运行时语义，类型系统表达不了。

所以与其追求编译期解决，不如换个思路——**能不能从架构上让死锁变得不可能，而不是靠纪律去避免？**

最彻底的办法就是一个原则：**每个 Context 只允许有一把锁。**

```rust
// 只有一把锁，结构上不可能死锁
struct SchedulerContext {
    state: Arc<Mutex<SchedulerState>>,
}
```

这条规则不需要编译器帮忙，CI 里写个脚本 grep 一下就行——任何 Context struct 里出现两个 `Mutex` 就报错。十行脚本顶过任何复杂的类型体操。

如果业务上确实需要跨多个 Context 协调，那这个协调点本身不持有锁，只按固定顺序调用各个 Context：

```rust
// 不是 Context，没有锁，只做编排
struct SchedulerService {
    sched: SchedulerContext,
    store: StoreContext,
}

impl SchedulerService {
    async fn add_task(&self, task: Task) {
        let snapshot = self.sched.enqueue(task).await;
        // sched 的锁已经释放了
        self.store.save(snapshot).await;
        // store 的锁也释放了
        // 全程没有同时持有两把锁
    }
}
```

整套规范就三条：

第一，核心对象正常命名，`&mut self`，可以 async，不碰锁。

第二，Context 是并发外壳，**只允许一把锁**，CI 自动检查。

第三，跨 Context 协调用 Service，Service 自己不持有锁，只按顺序调 Context 方法。

死锁需要"同时持有两把锁"。每一层都拿不到两把锁，死锁在架构上就不存在了。不靠编译器，不靠开发者自觉，靠结构。