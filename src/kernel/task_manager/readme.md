# TaskManager 服务

TaskManager是一个通用的任务管理服务，用于管理长时间运行的任务，类似于传统操作系统中的计划任务组件。它提供了一个统一的接口来创建、监控和管理各种类型的任务。

## 功能特点

- 创建和管理不同类型的任务
- 跟踪任务进度和状态
- 存储任务相关的数据和错误信息
- 支持任务暂停、恢复和取消
- 提供事件监听机制，可以监听任务状态变化
- 通过RPC接口提供服务，支持微服务架构

## 任务状态流转

任务可以有以下状态：

- `Pending`: 任务已创建但尚未开始
- `Running`: 任务正在运行
- `Paused`: 任务已暂停
- `Completed`: 任务已完成
- `Failed`: 任务失败
- `WaitingForApproval`: 任务完成但等待审核/批准

## API 接口

### Rust服务端API

TaskManager服务提供以下RPC方法：

- `create_task`: 创建新任务
- `get_task`: 获取任务信息
- `list_tasks`: 列出任务
- `update_task_status`: 更新任务状态
- `update_task_progress`: 更新任务进度
- `update_task_error`: 更新任务错误信息
- `update_task_data`: 更新任务数据
- `delete_task`: 删除任务

### TypeScript客户端API

TaskManager客户端提供以下方法：

- `createTask(name, task_type, app_name, data?)`: 创建新任务
- `getTask(id)`: 获取任务信息
- `listTasks(filter?)`: 列出任务
- `updateTaskStatus(id, status)`: 更新任务状态
- `updateTaskProgress(id, completed_items, total_items)`: 更新任务进度
- `updateTaskError(id, error_message)`: 更新任务错误信息
- `updateTaskData(id, data)`: 更新任务数据
- `deleteTask(id)`: 删除任务
- `pauseTask(id)`: 暂停任务
- `resumeTask(id)`: 恢复任务
- `completeTask(id)`: 完成任务
- `markTaskAsWaitingForApproval(id)`: 标记任务为等待审核
- `markTaskAsFailed(id, error_message)`: 标记任务为失败
- `pauseAllRunningTasks()`: 暂停所有运行中的任务
- `resumeLastPausedTask()`: 恢复最后一个暂停的任务

## 使用示例

### 创建发布任务

```typescript
// 创建发布任务
const taskData = {
  pkg_list: ["pkg1", "pkg2", "pkg3"]
};
const taskId = await taskManager.createTask(
  "发布包任务", 
  "publish", 
  "package-manager", 
  taskData
);

// 开始执行任务
await taskManager.updateTaskStatus(taskId, TaskStatus.Running);

// 检查依赖
try {
  // 检查pkg_list的各种deps是否已经在当前index-meta-db中存在
  // ...检查逻辑...
} catch (error) {
  // 检查失败，写入错误信息
  await taskManager.markTaskAsFailed(taskId, error.message);
}

// 获取待下载的chunklist
const chunkList = ["chunk1", "chunk2", "chunk3"]; // 示例
await taskManager.updateTaskData(taskId, {
  pkg_list: taskData.pkg_list,
  chunk_list: chunkList
});

// 下载chunk并更新进度
for (let i = 0; i < chunkList.length; i++) {
  try {
    // 下载chunk的逻辑
    // ...
    await taskManager.updateTaskProgress(taskId, i + 1, chunkList.length);
  } catch (error) {
    await taskManager.markTaskAsFailed(taskId, `下载chunk失败: ${error.message}`);
    break;
  }
}

// 下载完成，标记为等待审核
await taskManager.markTaskAsWaitingForApproval(taskId);

// 最终发布
// 合并发布任务里包含的pkg_list到local-wait-meta
// ...发布逻辑...
await taskManager.completeTask(taskId);
```

## 事件监听

可以通过添加事件监听器来监听任务状态变化：

```typescript
taskManager.addTaskEventListener(async (event, data) => {
  console.log(`Task event: ${event}`, data);
  
  // 可以根据不同的事件类型执行不同的操作
  if (event === 'task_status_updated' && data.status === TaskStatus.WaitingForApproval) {
    // 通知管理员审核
    // ...
  }
});
```
