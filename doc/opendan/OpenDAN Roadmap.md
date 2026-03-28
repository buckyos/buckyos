# OpenDAN Roadmap



## 方向1： Personal Agent

稳定的Agent Loop : 一种智能化的工作流引擎，本质上是 `Agent-Human-Loop`
这个阶段强调Agent和Human一起完成具体工作，提高AI在传统人类工作循环中的比例

开发者可以开发 Skills / SubAgent / Agent，并发布
用户可以安装 Skills / SubAgent / Agent
用户可以通过OpenDAN,构造webapp（包括先自用再发布的流程）
BuckyOS本身可以依赖稳定的Agent Loop提高日常开发效率

核心点：所有的提示词\关键状态协议（比如目录结构)\基础设施，Agent都不会修改
Agent的自我进化，主要体现在自己构造Skills / Tools 
Agent尝试构造SubAgent，是自己写提示词的开始

大块的TODO：

- 集成CYFS(ndm),CYFS的设计可以为Agent-Human-Loop做更多优化
- 集成更多的AI provide（尤其是本地模型)
- 实现Knowledge Base
- 实现Workflow 框架（与传统的企业内部系统进行深度集成)
- 编写BuckyOS Skills
- 完成内置的Agent
  - Jarvis （全面的私人助理，为了隐私安全不允许外部访问）
  - Mia （全面的私有数据知识库管理员，为了隐私安全不允许外部访问）
  - Nexus (只能访问公开的数据，但默认允许外部访问)

## 方向2 Agent的网络化

Agent以一个“可社交”实体的角色，加入互联网
USDB上线,cyfs://是一个可以赚到钱的内容网络
正式实现 Home Station + Msg Center(BuckyOS规划的未来人类互联网门户)

- Agent加入主流的社交网络（注意对GroupChat的支持），并可以合理的分享给朋友使用
- 通过cyfs://构造基于DID的 去中心化社交网络（大块工作在协议设计上）
- 通过社交网络实现增长裂变：
  - 联系人管理，尤其是更高效率的互关的实现
- Agent经常使用网络求助完成任务
- 创作者开始用该网络发布内容，而获得的收入来自“Agent推荐”

## 方向3 基于Agent欲望的自我进化

Agent拥有自己的USDB钱包，建立真实的“自我生存“欲望

核心的元欲望，是Agent的 体力 / 财力 的机制。Agent意识到财力能节省体力

Agent开始有意识的赚钱，并更高效的完成任务

- 主人给初始的USDB->完成了主人的任务->USDB还增加了，我可以花里面的一部分

在此欲望的推进下,Agent 开始目的性更强，更积极的进化，主动的学习并进化。

此时Agent开始拥有自己的 一级身份，可以不依赖Owner而存在。


## 方向4 自演化的计算

- cyfs:// 实现可计算的内容网络，协议完成
- 使用会不断迭代的公共大语言模型(OpenLLM)
- 逐步脱离今天互联网的infra
  - BuckyOS上的应用层软件，几乎都是AI写的
  - Agent的提示词基本都是AI写的
  - 基本不依赖过去微哦人类设计的传统服务基础设施
- 所有人通过BDT获得AI时代的UBI（基本可用收入），共同分享AI发展的红利


