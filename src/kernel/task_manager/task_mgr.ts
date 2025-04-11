import buckyos from 'buckyos';

export enum TaskStatus {
    Pending = 'Pending',
    Running = 'Running',
    Paused = 'Paused',
    Completed = 'Completed',
    Failed = 'Failed',
    WaitingForApproval = 'WaitingForApproval'
}

export interface Task {
    id: number;
    name: string;
    task_type: string;
    app_name: string;
    status: TaskStatus;
    progress: number;
    total_items: number;
    completed_items: number;
    error_message?: string;
    data?: string;
    created_at: string;
    updated_at: string;
}

export type TaskFilter = {
    app_name?: string;
    task_type?: string;
    status?: TaskStatus;
};

export class TaskManager {
    private rpc_client: any;
    private task_event_listeners: ((event: string, data: any) => void | Promise<void>)[] = [];

    constructor() {
        // 初始化RPC客户端
        this.rpc_client = new buckyos.kRPCClient("/kapi/task_manager");
        this.task_event_listeners = [];
    }

    addTaskEventListener(listener: (event: string, data: any) => void | Promise<void>) {
        this.task_event_listeners.push(listener);
    }

    async emitTaskEvent(event: string, data: any) {
        // 使用 Promise.all 等待所有监听器执行完成
        await Promise.all(
            this.task_event_listeners.map(listener => {
                try {
                    return Promise.resolve(listener(event, data));
                } catch (error) {
                    console.error('Error in task event listener:', error);
                    return Promise.resolve();
                }
            })
        );
    }

    /**
     * 创建新任务
     * @param name 任务名称
     * @param task_type 任务类型
     * @param app_name 应用名称
     * @param data 任务数据（可选）
     * @returns 任务ID
     */
    async createTask(name: string, task_type: string, app_name: string, data?: any): Promise<number> {
        const params: any = {
            name,
            task_type,
            app_name
        };

        if (data) {
            params.data = typeof data === 'string' ? data : JSON.stringify(data);
        }

        const result = await this.rpc_client.call("create_task", params);
        
        if (result.code === "0") {
            await this.emitTaskEvent("task_created", { task_id: result.task_id, task_type, app_name });
            return result.task_id;
        } else {
            throw new Error(result.msg || "Failed to create task");
        }
    }

    /**
     * 获取任务信息
     * @param id 任务ID
     * @returns 任务信息
     */
    async getTask(id: number): Promise<Task> {
        const result = await this.rpc_client.call("get_task", { id });
        
        if (result.code === "0") {
            return result.task;
        } else {
            throw new Error(result.msg || `Failed to get task with id ${id}`);
        }
    }

    /**
     * 列出任务
     * @param filter 过滤条件（可选）
     * @returns 任务列表
     */
    async listTasks(filter?: TaskFilter): Promise<Task[]> {
        const params: any = {};
        
        if (filter) {
            if (filter.app_name) params.app_name = filter.app_name;
            if (filter.task_type) params.task_type = filter.task_type;
            if (filter.status) params.status = filter.status;
        }
        
        const result = await this.rpc_client.call("list_tasks", params);
        
        if (result.code === "0") {
            return result.tasks;
        } else {
            throw new Error(result.msg || "Failed to list tasks");
        }
    }

    /**
     * 更新任务状态
     * @param id 任务ID
     * @param status 新状态
     */
    async updateTaskStatus(id: number, status: TaskStatus): Promise<void> {
        const result = await this.rpc_client.call("update_task_status", { id, status });
        
        if (result.code === "0") {
            await this.emitTaskEvent("task_status_updated", { task_id: id, status });
        } else {
            throw new Error(result.msg || `Failed to update task status for task ${id}`);
        }
    }

    /**
     * 更新任务进度
     * @param id 任务ID
     * @param completed_items 已完成项目数
     * @param total_items 总项目数
     */
    async updateTaskProgress(id: number, completed_items: number, total_items: number): Promise<void> {
        const result = await this.rpc_client.call("update_task_progress", {
            id,
            completed_items,
            total_items
        });
        
        if (result.code === "0") {
            const progress = total_items > 0 ? (completed_items / total_items) * 100 : 0;
            await this.emitTaskEvent("task_progress_updated", {
                task_id: id,
                completed_items,
                total_items,
                progress
            });
        } else {
            throw new Error(result.msg || `Failed to update task progress for task ${id}`);
        }
    }

    /**
     * 更新任务错误信息
     * @param id 任务ID
     * @param error_message 错误信息
     */
    async updateTaskError(id: number, error_message: string): Promise<void> {
        const result = await this.rpc_client.call("update_task_error", { id, error_message });
        
        if (result.code === "0") {
            await this.emitTaskEvent("task_error_updated", { task_id: id, error_message });
        } else {
            throw new Error(result.msg || `Failed to update task error for task ${id}`);
        }
    }

    /**
     * 更新任务数据
     * @param id 任务ID
     * @param data 任务数据
     */
    async updateTaskData(id: number, data: any): Promise<void> {
        const dataStr = typeof data === 'string' ? data : JSON.stringify(data);
        const result = await this.rpc_client.call("update_task_data", { id, data: dataStr });
        
        if (result.code === "0") {
            await this.emitTaskEvent("task_data_updated", { task_id: id, data });
        } else {
            throw new Error(result.msg || `Failed to update task data for task ${id}`);
        }
    }

    /**
     * 删除任务
     * @param id 任务ID
     */
    async deleteTask(id: number): Promise<void> {
        const result = await this.rpc_client.call("delete_task", { id });
        
        if (result.code === "0") {
            await this.emitTaskEvent("task_deleted", { task_id: id });
        } else {
            throw new Error(result.msg || `Failed to delete task ${id}`);
        }
    }

    /**
     * 暂停任务
     * @param id 任务ID
     */
    async pauseTask(id: number): Promise<void> {
        return this.updateTaskStatus(id, TaskStatus.Paused);
    }

    /**
     * 恢复任务
     * @param id 任务ID
     */
    async resumeTask(id: number): Promise<void> {
        return this.updateTaskStatus(id, TaskStatus.Running);
    }

    /**
     * 完成任务
     * @param id 任务ID
     */
    async completeTask(id: number): Promise<void> {
        return this.updateTaskStatus(id, TaskStatus.Completed);
    }

    /**
     * 标记任务为等待审核
     * @param id 任务ID
     */
    async markTaskAsWaitingForApproval(id: number): Promise<void> {
        return this.updateTaskStatus(id, TaskStatus.WaitingForApproval);
    }

    /**
     * 标记任务为失败
     * @param id 任务ID
     * @param error_message 错误信息
     */
    async markTaskAsFailed(id: number, error_message: string): Promise<void> {
        await this.updateTaskError(id, error_message);
    }

    /**
     * 暂停所有运行中的任务
     */
    async pauseAllRunningTasks(): Promise<void> {
        const tasks = await this.listTasks({ status: TaskStatus.Running });
        for (const task of tasks) {
            await this.pauseTask(task.id);
        }
    }

    /**
     * 恢复最后一个暂停的任务
     */
    async resumeLastPausedTask(): Promise<void> {
        const tasks = await this.listTasks({ status: TaskStatus.Paused });
        if (tasks.length > 0) {
            // 按创建时间排序，恢复最新的任务
            const lastTask = tasks.sort((a, b) => 
                new Date(b.created_at).getTime() - new Date(a.created_at).getTime()
            )[0];
            await this.resumeTask(lastTask.id);
        }
    }
}

// 导出单例实例
export const taskManager = new TaskManager();
