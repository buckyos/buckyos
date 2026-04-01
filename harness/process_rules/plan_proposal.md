
# 规划任务的处理规则

目标：基于当前feature的现有文档，进行开发任务规划,规划结果保存在tasklist.md。
tasklist.md是后续大量开发工作的起点，高质量的完成规划将创造极大的价值。

## 确认当前任务feature的状态

工作目录必须在Agent自己的git worktree,并已经checkout feature/$feaature_name 分支.如果该分支不存在，自动创建。
当前工作的主要交付物保存在$REPO_ROOT/proposals/$feature_name/tasklist.md 文件里。如果该文件已经存在，说明规划任务已经完成，应立刻结束当前任务。

## 充分理解需求
**如果feature文档目录下包含了太多的文档，或则文档太长（超过TokenLimit的1半），则明确的拒绝该任务。**
- 通常先看当前feature文档目录下是否存在 proposal.md或glal.md, 这类文件完整的说明了这个feature的核心目的
- 再看有没有 包含"PRD"的.md文件，对于有UI开发的feature,至少要有一个PRD文件。
- 以资深架构师的角度阅读上述文档，识别对实施有影响的模糊部分。
- 主动反问用户要求补充信息，直到所有的需求和目标都清晰无歧义。

## 充分理解开发流程和BuckyOS的现有实现

**要求用户给出本feature开发需要遵循的开发流程** 

- 关注开发流程里的checkpoint ,每两个checkpoint之间至少要规划一个开发任务
- 关注开发流程里的负责人切换，一个task只有一个负责人
- 如果开发流程里给出了task模板，则优先基于能“套上模板”来切分Task

**BuckyOS是典型的分布式OS，充分探索现有系统里已有的类似模块的设计**

## 分多次创建task
- 优先创建可以套用task template的任务。如开发流程未指定Task Template，则尝试在 `$REPO_ROOT/harness/process_rules/task_templates` 目录下匹配一个合适的模板
- 没有合适的模板的情况下，基于你的架构经验创建`目标清晰、结果可以验证、能实现`的开发任务。
- 写入 task-$name-$number.md 后，说明该task创建完成

## 完成的定义
- 查看`$REPO_ROOT/proposals/$feature_name/` 目录下的所有task开头的.md文件，得到当前任务列表
- 针对该列表进行一次整体的Review，确认所有的task都有清晰的任务目标
- 构造tasklist.md


