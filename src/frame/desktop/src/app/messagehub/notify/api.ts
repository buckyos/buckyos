import { buckyos, taskMgrErrorCode, TASK_ERR_NOT_FOUND, TASK_ERR_PERMISSION_DENIED } from 'buckyos'
import { getTaskWatchSource } from '../conversation/tasks'
import { ackNotify, dismissNotify, fetchAccountUsername, getNotify, MessageHubApiError } from '../datamodel/sessionApi'
import { parseHumanTaskPrompt } from '../protocol/msgobj'
import type { NotifyView } from './model'
import type { NotifyBackend } from './watch'

/** The prompt comes from an App: read it the way a card renders it. */
function normalized(view: NotifyView): NotifyView {
  return { ...view, prompt: parseHumanTaskPrompt(view.prompt), source: view.source ?? {} }
}

/**
 * msg-center `notify.get / ack / dismiss` and the TaskMgr side of an answer:
 * `get_task` and `commit_result` with `expected_revision`, using the
 * logged-in user's own permissions (the user is the task's assignee). The
 * task's `/task_mgr/<id>` kevent shares MessageHub's one event stream.
 */
export function createApiNotifyBackend(): NotifyBackend {
  let username: Promise<string> | null = null
  return {
    get: notifyId => getNotify(notifyId).then(normalized),
    ack: notifyId => ackNotify(notifyId).then(normalized),
    dismiss: notifyId => dismissNotify(notifyId).then(normalized),
    async getTask(taskId) {
      const task = await buckyos.getTaskManagerClient().getTask(taskId)
      return { revision: task.revision, phase: task.phase, result: task.result }
    },
    async commitResult(taskId, result, expectedRevision) {
      await buckyos.getTaskManagerClient().commitResult({ task_id: taskId, result, expected_revision: expectedRevision })
    },
    taskErrorCode: error => taskMgrErrorCode(error),
    failure(error) {
      if (error instanceof MessageHubApiError) return error.kind === 'not_found' ? 'missing' : error.kind === 'permission_denied' ? 'denied' : 'error'
      const code = taskMgrErrorCode(error)
      return code === TASK_ERR_PERMISSION_DENIED ? 'denied' : code === TASK_ERR_NOT_FOUND ? 'missing' : 'error'
    },
    subscribe(_notifyId, taskId, onHint) {
      return taskId ? getTaskWatchSource().subscribe('task', taskId, onHint) : () => {}
    },
    submitter() {
      username ??= fetchAccountUsername().catch(error => { username = null; throw error })
      return username
    },
    now: () => Date.now(),
  }
}
