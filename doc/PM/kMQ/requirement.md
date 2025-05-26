# kMQ 需求文档

kMQ是BuckyOS Kernel Message Queue的缩写。提供了内核级别的消息队列

## 功能描述

消息队列是系统中的一种重要的基础资源
基本使用思路是:

//app1 创建消息队列
create_msg_queue(queue_id,queue_config)

//app2 往消息队列中放消息
post_msg(queue_id,msg)

//app1 弹出消息处理
msg = pop_msg(queue_id)
reply_msg(msg,msg_result)

//app2 获得消息的回复
msg_result = get_msg_reply(msg)

//app1 销毁消息队列
close_msg_queue(queue_id)


- 调用post_msg时，kMQ服务会持久化保障msg,并保障顺序性
- msg_queue会消耗app1的存储配额，当msg_queue满后，无法post消息
- kMQ是高可用服务，不会因为系统的单点故障失效，系统也尽力保障起数据可用性
- 有权限的用户（或系统）可以对kMQ的配额和拥塞进行控制。作为系统的基础设施可透明的调整系统负载分配
- msq_queue也可以配置成纯内存模式（只保障顺序不保障序列化），可用于某些性能特化场景

## 核心接口设计
- 设计msg_queue service,基于NATS+jetstream实现
- 先抽象出trait,方便在有需要的场景透明的替换底层实现（比如在物联网场合基于sqlite进行单节点实现）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfig {
    pub max_size: usize,
    pub persistence: bool,
    pub retention_period: Option<u64>, // in seconds
    pub max_message_size: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub queue_id: String,
    pub content: Vec<u8>,
    pub timestamp: u64,
    pub reply_to: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageReply {
    pub message_id: Uuid,
    pub result: Vec<u8>,
    pub timestamp: u64,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueStats {
    pub queue_id: String,
    pub message_count: usize,
    pub max_size: usize,
    pub used_storage: usize,
    pub created_at: u64,
    pub last_accessed: u64,
} 

#[async_trait]
pub trait MsgQueueBackend: Send + Sync {
    async fn create_queue(&self, queue_id: &str, config: QueueConfig) -> Result<(), MsgQueueError>;
    async fn update_queue_config(&self, queue_id: &str, config: QueueConfig) -> Result<(), MsgQueueError>;
    async fn delete_queue(&self, queue_id: &str) -> Result<(), MsgQueueError>;
    
    async fn post_message(&self, queue_id: &str, message: Message) -> Result<(), MsgQueueError>;
    async fn pop_message(&self, queue_id: &str) -> Result<Option<Message>, MsgQueueError>;
    
    async fn get_message_reply(&self, message_id: &str) -> Result<Option<MessageReply>, MsgQueueError>;
    async fn reply_to_message(&self, reply: MessageReply) -> Result<(), MsgQueueError>;
    
    async fn get_queue_stats(&self, queue_id: &str) -> Result<QueueStats, MsgQueueError>;

} 
```

（基于 功能描述 发给AI调研后，得出结论使用NATS + jetstream, 然后根据例子手工调整接口定义）
（service都需要通过kRPC向外提供服务，因此这里也要消息的思考这些接口不会在kRPC化后带来巨大的劣化）


## 添加单元测试
在curosr里，可以让AI基于接口设计先编写测试代码。这可以减少实现代码对LLM windows的占用，提高AI实现单元测试的正确性。

Review单元测试后的实现后，可以手工添加一些测试用例，并要求AI实现。

## 交给实现核心接口
在有上述接口的设计的情况下，要求AI编写实现。
编写完成后，可通过运行单元测试来判断AI是否已经正确实现
单元测试依赖的外部任务，应在运行单元测试前先准备好（包括进行必要的重置）

## 使用kRPC对外提供服务



## 基于AI进行开发的一些关键工程要点
- 核心是多阶段，以人为主，AI辅助作体力活
- AI的宽知识面非常适合进行架构设计的讨论
- 当AI开始编码时，应能尽量减少单词AI生成代码的规模，并能在工程上固定一个阶段的AI生成的结果


第一阶段：与AI讨论架构设计，完成requiment.md

应保留该阶段的对话记录，作为架构设计文档的一部分
相比传统的架构文档，与AI聊天讨论架构设计能包含更多的“为什么不用另一个方案”，和如何进行trade off的关键思考。

第二阶段：在AI的辅助下完成框架编码


- 根据架构设计结果，以AI建立框架，手工辅助的方法完成核心接口和方法的编码。对rust来说还有使用模式的问题，因此也要在这个阶段的编码里体现一些关键的生命周期管理的概念
- 基于未实现的核心接口构造测试用例（构建使用场景）
- 编译通过后，进入框架Review阶段，Review通过后进入下一个阶段


第三阶段: AI辅助实现
有了上面的成果，这里就可以把AI的工作限定在“实现某个函数”上了。
此时的工作循环是 让AI实现函数->运行测试用例->改进测试用例
本阶段完成后，可以确保提交的代码至少能通过cargo test
此时可以发起PR了



