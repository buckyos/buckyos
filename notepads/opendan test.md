# OpenDAN 集成测试

## AICC层

- 所有的模型都用一遍（需要根据模型的典型能力构造请求）
- 协议转换正确，构造所有可能的AICC请求，查看是否能转成合理的相关协议的请求并能返回
- 逻辑目录的组织是否合理

## AgentTool
- 检查提示词是否简洁清晰

## LLM Context

### 主动测试旁路流程
- LLM Compress
- update_session_topic
- 意图引擎测试



## AgentSession

- 提示词渲染，能基于一个构造的AgentSession，验证所有的渲染都对
- 能构造一个快照进行推理，而不用走复杂的前置流程

### Chat Session

- 复杂的read测试
- 文件格式测试
- try-create-work-session 测试（旁路LLM)
- 访问Host机测试

### Work Session

- 从状态机的角度，覆盖集中典型的状态切换
- 根据需求，完成网页开发的任务
- 使用AIGC工具，完成素材加工的任务
- 编写spider的任务


### Self-Check 

- 机械触发
- 能否

### Group-Chat


### Self-Improve