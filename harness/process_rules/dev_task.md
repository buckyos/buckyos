# 书面开发任务的处理规则

## 确认是否在正确的工作目录

工作目录必须在Agent自己的git worktree,并已经checkout feature/$feaature_name/$task_name 分支.如果该分支不存在，自动创建。

当前任务关联的文档目录是 `$REPO_ROOT/proposals/$feature_name/$task_name`, 在处理过程中如有需要，可以自由的在该目录中添加文档。

## 检查依赖

留意书面开发中明确声明的任务依赖。如果依赖另一个保存在`$REPO_ROOT/proposals/$feature_name/` 目录下的任务，则通过读取其任务书判断其是否完成。

## 不停迭代直到完成

Step1. 阅读任务的书面要求，清楚理解“何为完成”
Step2. 理解现有实现，确认到“完成的差距“
Step3. 动手实现，目的是逼近完成。每次实现时，都应尝试载入合适的skills
Setp4. 根据当前任务的领域，进行基础的 lint和unit test，让代码跑起来
Step5. 回到Step2，直到确认完成为止

## 处理副作用

上述开发工作完成后，Review现有分支上的改动涉及的文件:
阅读`$REPO_ROOT\harness\trigger_rules.md`,判断是否有触发高危修改。如果触发了可以选择

1. 按该文档要求的对应处理流程进行附加修改（尤其是对关键协议性文档的更新)
2. 选择更换到不触发高危修改的方案

## 宣告完成

当达到任务书要求的目标后:
- 修改任务书，添加任务状态为: 开发完成，等待REVIEW。
- 在当前分支上git commit，并通知用户worktree的目录和工作分支名。
