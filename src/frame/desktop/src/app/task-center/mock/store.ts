/* ── TaskCenter mock store ── */

import type {
  Task,
  TaskStatus,
  TaskType,
  TaskSource,
  SystemNotification,
  SystemEvent,
} from './types'

function makeTask(
  overrides: Partial<Task> & Pick<Task, 'taskId' | 'title' | 'type' | 'status'>,
): Task {
  const now = new Date().toISOString()
  return {
    rootTaskId: overrides.taskId,
    parentTaskId: null,
    source: 'system',
    summary: '',
    createdAt: now,
    updatedAt: now,
    startedAt: overrides.status !== 'pending' ? now : null,
    endedAt:
      overrides.status === 'completed' || overrides.status === 'failed' || overrides.status === 'cancelled'
        ? now
        : null,
    progress: null,
    schemaType: null,
    payload: {},
    children: [],
    ...overrides,
  }
}

const seedTasks: Task[] = [
  makeTask({
    taskId: 'task-001',
    rootTaskId: 'task-001',
    title: 'Install BuckyOS System Update v2.4.1',
    summary: 'Downloading and installing system update package...',
    type: 'install',
    status: 'running',
    source: 'system',
    progress: 67,
    createdAt: '2026-04-04T08:30:00Z',
    updatedAt: '2026-04-04T09:15:00Z',
    startedAt: '2026-04-04T08:31:00Z',
    children: [
      makeTask({
        taskId: 'task-001-1',
        rootTaskId: 'task-001',
        parentTaskId: 'task-001',
        title: 'Download update package',
        type: 'download',
        status: 'completed',
        progress: 100,
        summary: '1.2 GB downloaded',
      }),
      makeTask({
        taskId: 'task-001-2',
        rootTaskId: 'task-001',
        parentTaskId: 'task-001',
        title: 'Verify package integrity',
        type: 'one-time',
        status: 'completed',
        progress: 100,
        summary: 'SHA-256 verified',
      }),
      makeTask({
        taskId: 'task-001-3',
        rootTaskId: 'task-001',
        parentTaskId: 'task-001',
        title: 'Apply system patches',
        type: 'one-time',
        status: 'running',
        progress: 45,
        summary: 'Applying patch 3 of 7...',
      }),
    ],
  }),
  makeTask({
    taskId: 'task-002',
    rootTaskId: 'task-002',
    title: 'Sync Photos to Cloud Backup',
    summary: 'Syncing 2,847 photos to remote storage...',
    type: 'sync',
    status: 'running',
    source: 'user',
    progress: 34,
    createdAt: '2026-04-04T07:00:00Z',
    updatedAt: '2026-04-04T09:10:00Z',
    startedAt: '2026-04-04T07:01:00Z',
  }),
  makeTask({
    taskId: 'task-003',
    rootTaskId: 'task-003',
    title: 'AI Model Fine-tuning Job',
    summary: 'Training custom model on local dataset',
    type: 'workflow',
    status: 'running',
    source: 'agent',
    progress: 12,
    createdAt: '2026-04-03T22:00:00Z',
    updatedAt: '2026-04-04T09:00:00Z',
    startedAt: '2026-04-03T22:05:00Z',
    children: [
      makeTask({
        taskId: 'task-003-1',
        rootTaskId: 'task-003',
        parentTaskId: 'task-003',
        title: 'Data preprocessing',
        type: 'one-time',
        status: 'completed',
        progress: 100,
        summary: '15,000 samples processed',
      }),
      makeTask({
        taskId: 'task-003-2',
        rootTaskId: 'task-003',
        parentTaskId: 'task-003',
        title: 'Model training - Epoch 2/15',
        type: 'one-time',
        status: 'running',
        progress: 12,
        summary: 'Loss: 0.342, Accuracy: 78.5%',
      }),
    ],
  }),
  makeTask({
    taskId: 'task-004',
    rootTaskId: 'task-004',
    title: 'Download Large Dataset',
    summary: 'Downloading ImageNet subset (48 GB)',
    type: 'download',
    status: 'failed',
    source: 'user',
    progress: 82,
    createdAt: '2026-04-03T14:00:00Z',
    updatedAt: '2026-04-03T18:30:00Z',
    startedAt: '2026-04-03T14:02:00Z',
    endedAt: '2026-04-03T18:30:00Z',
    payload: { error: 'Network timeout after 3 retries. Last chunk failed at offset 39.4 GB.' },
  }),
  makeTask({
    taskId: 'task-005',
    rootTaskId: 'task-005',
    title: 'Install App: Notion Connector',
    summary: 'Successfully installed Notion Connector v1.3.0',
    type: 'install',
    status: 'completed',
    source: 'user',
    progress: 100,
    createdAt: '2026-04-03T10:00:00Z',
    updatedAt: '2026-04-03T10:05:00Z',
    startedAt: '2026-04-03T10:00:30Z',
    endedAt: '2026-04-03T10:05:00Z',
  }),
  makeTask({
    taskId: 'task-006',
    rootTaskId: 'task-006',
    title: 'Scheduled Backup: Weekly Full',
    summary: 'Next run scheduled for 2026-04-07 02:00',
    type: 'scheduled',
    status: 'completed',
    source: 'system',
    progress: 100,
    createdAt: '2026-03-31T02:00:00Z',
    updatedAt: '2026-03-31T03:45:00Z',
    startedAt: '2026-03-31T02:00:00Z',
    endedAt: '2026-03-31T03:45:00Z',
  }),
  makeTask({
    taskId: 'task-007',
    rootTaskId: 'task-007',
    title: 'Directory Sync: /media → NAS',
    summary: 'Cancelled by user',
    type: 'sync',
    status: 'cancelled',
    source: 'user',
    createdAt: '2026-04-02T16:00:00Z',
    updatedAt: '2026-04-02T16:10:00Z',
    startedAt: '2026-04-02T16:00:30Z',
    endedAt: '2026-04-02T16:10:00Z',
  }),
  makeTask({
    taskId: 'task-008',
    rootTaskId: 'task-008',
    title: 'Agent Workflow: Data Pipeline',
    summary: 'Waiting for user authorization',
    type: 'workflow',
    status: 'paused',
    source: 'agent',
    progress: 60,
    createdAt: '2026-04-04T06:00:00Z',
    updatedAt: '2026-04-04T08:00:00Z',
    startedAt: '2026-04-04T06:05:00Z',
  }),
]

const seedNotifications: SystemNotification[] = [
  {
    id: 'notif-001',
    source: 'system',
    title: 'Agent Authorization Required',
    summary: 'Agent "DataBot" is requesting access to /private/documents. Approve or deny this request.',
    severity: 'critical',
    createdAt: '2026-04-04T08:45:00Z',
    actions: ['approve', 'reject'],
    handled: false,
  },
  {
    id: 'notif-002',
    source: 'system',
    title: 'Disk Space Warning',
    summary: 'Volume /data has only 8% free space remaining (12.4 GB of 160 GB).',
    severity: 'warning',
    createdAt: '2026-04-04T07:30:00Z',
    actions: ['dismiss'],
    handled: false,
  },
  {
    id: 'notif-003',
    source: 'system',
    title: 'Security Policy Update',
    summary: 'New security policy requires re-authentication for all active sessions within 24 hours.',
    severity: 'info',
    createdAt: '2026-04-04T06:00:00Z',
    actions: ['confirm'],
    handled: false,
  },
]

const seedEvents: SystemEvent[] = [
  {
    eventId: 'evt-001',
    eventType: 'task_created',
    source: 'system',
    relatedRootTaskId: 'task-001',
    relatedTaskId: 'task-001',
    title: 'Task Created: Install BuckyOS System Update v2.4.1',
    summary: 'System update installation initiated',
    occurredAt: '2026-04-04T08:30:00Z',
    actionState: 'none',
    actionAt: null,
    payload: {},
  },
  {
    eventId: 'evt-002',
    eventType: 'task_milestone',
    source: 'system',
    relatedRootTaskId: 'task-001',
    relatedTaskId: 'task-001-1',
    title: 'Download Complete: System Update Package',
    summary: 'Package download finished, proceeding to verification',
    occurredAt: '2026-04-04T09:00:00Z',
    actionState: 'none',
    actionAt: null,
    payload: {},
  },
  {
    eventId: 'evt-003',
    eventType: 'task_created',
    source: 'user',
    relatedRootTaskId: 'task-002',
    relatedTaskId: 'task-002',
    title: 'Task Created: Sync Photos to Cloud Backup',
    summary: 'User initiated photo sync job',
    occurredAt: '2026-04-04T07:00:00Z',
    actionState: 'none',
    actionAt: null,
    payload: {},
  },
  {
    eventId: 'evt-004',
    eventType: 'task_failed',
    source: 'system',
    relatedRootTaskId: 'task-004',
    relatedTaskId: 'task-004',
    title: 'Task Failed: Download Large Dataset',
    summary: 'Network timeout after 3 retries at 82% progress',
    occurredAt: '2026-04-03T18:30:00Z',
    actionState: 'none',
    actionAt: null,
    payload: {},
  },
  {
    eventId: 'evt-005',
    eventType: 'task_completed',
    source: 'user',
    relatedRootTaskId: 'task-005',
    relatedTaskId: 'task-005',
    title: 'Task Completed: Install App: Notion Connector',
    summary: 'Notion Connector v1.3.0 installed successfully',
    occurredAt: '2026-04-03T10:05:00Z',
    actionState: 'none',
    actionAt: null,
    payload: {},
  },
  {
    eventId: 'evt-006',
    eventType: 'notification_created',
    source: 'system',
    relatedRootTaskId: null,
    relatedTaskId: null,
    title: 'Agent Authorization Required',
    summary: 'Agent "DataBot" requesting access to /private/documents',
    occurredAt: '2026-04-04T08:45:00Z',
    actionState: 'none',
    actionAt: null,
    payload: { notificationId: 'notif-001' },
  },
  {
    eventId: 'evt-007',
    eventType: 'notification_created',
    source: 'system',
    relatedRootTaskId: null,
    relatedTaskId: null,
    title: 'Disk Space Warning',
    summary: 'Volume /data at 92% capacity',
    occurredAt: '2026-04-04T07:30:00Z',
    actionState: 'none',
    actionAt: null,
    payload: { notificationId: 'notif-002' },
  },
  {
    eventId: 'evt-008',
    eventType: 'task_completed',
    source: 'system',
    relatedRootTaskId: 'task-006',
    relatedTaskId: 'task-006',
    title: 'Task Completed: Scheduled Backup: Weekly Full',
    summary: 'Backup completed in 1h 45m',
    occurredAt: '2026-03-31T03:45:00Z',
    actionState: 'none',
    actionAt: null,
    payload: {},
  },
  {
    eventId: 'evt-009',
    eventType: 'task_cancelled',
    source: 'user',
    relatedRootTaskId: 'task-007',
    relatedTaskId: 'task-007',
    title: 'Task Cancelled: Directory Sync: /media → NAS',
    summary: 'Cancelled by user',
    occurredAt: '2026-04-02T16:10:00Z',
    actionState: 'none',
    actionAt: null,
    payload: {},
  },
  {
    eventId: 'evt-010',
    eventType: 'task_milestone',
    source: 'agent',
    relatedRootTaskId: 'task-008',
    relatedTaskId: 'task-008',
    title: 'Task Paused: Agent Workflow: Data Pipeline',
    summary: 'Waiting for user authorization to proceed',
    occurredAt: '2026-04-04T08:00:00Z',
    actionState: 'none',
    actionAt: null,
    payload: {},
  },
]

export class TaskCenterMockStore {
  tasks: Task[] = seedTasks
  notifications: SystemNotification[] = seedNotifications
  events: SystemEvent[] = seedEvents

  getAllTasks(): Task[] {
    return this.tasks
  }

  getRunningTasks(): Task[] {
    return this.tasks.filter((t) => t.status === 'running' || t.status === 'paused')
  }

  getRecentFinishedTasks(): Task[] {
    return this.tasks.filter(
      (t) => t.status === 'completed' || t.status === 'failed' || t.status === 'cancelled',
    )
  }

  getTaskById(taskId: string): Task | null {
    for (const task of this.tasks) {
      if (task.taskId === taskId) return task
      for (const child of task.children) {
        if (child.taskId === taskId) return task // return root task
      }
    }
    return null
  }

  filterTasks(opts: {
    status?: TaskStatus
    type?: TaskType
    source?: TaskSource
    search?: string
  }): Task[] {
    return this.tasks.filter((t) => {
      if (opts.status && t.status !== opts.status) return false
      if (opts.type && t.type !== opts.type) return false
      if (opts.source && t.source !== opts.source) return false
      if (opts.search) {
        const q = opts.search.toLowerCase()
        if (
          !t.title.toLowerCase().includes(q) &&
          !t.taskId.toLowerCase().includes(q) &&
          !t.summary.toLowerCase().includes(q)
        )
          return false
      }
      return true
    })
  }

  getPendingNotifications(): SystemNotification[] {
    return this.notifications.filter((n) => !n.handled)
  }

  handleNotification(id: string, action: string): void {
    const notif = this.notifications.find((n) => n.id === id)
    if (notif) {
      notif.handled = true
      notif.handledAction = action as SystemNotification['handledAction']
      notif.handledAt = new Date().toISOString()
    }
  }

  getEvents(): SystemEvent[] {
    return [...this.events].sort(
      (a, b) => new Date(b.occurredAt).getTime() - new Date(a.occurredAt).getTime(),
    )
  }
}
