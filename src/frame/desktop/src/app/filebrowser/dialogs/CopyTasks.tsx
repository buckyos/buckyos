import { Button, Dialog, DialogActions, DialogContent, DialogTitle } from '@mui/material'
import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { folderOps, type BatchOptions, type OperationResult } from '../data/folderOps'
import { operationFeedback } from '../data/operationFeedback'
import { toUiError } from '../data/state'

export function CopyTasks({ ownerId, reveal }: { ownerId?: string; reveal: (path: string) => void }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const [tasks, setTasks] = useState<{ task_id: string; name: string; phase: string }[]>([])
  const [error, setError] = useState('')
  const refresh = async () => {
    try { setTasks(await folderOps().listCopies?.() ?? []); setError('') } catch (error) { setError(toUiError(error).fallback) }
  }
  const view = async (taskId: string, retry = false) => {
    if (operationFeedback.isRunning()) return
    setOpen(false)
    const operationKey = crypto.randomUUID()
    const controller = new AbortController()
    let currentId = taskId
    const options: BatchOptions = {
      signal: controller.signal,
      retryOf: retry ? taskId : undefined,
      onProgress: (results) => operationFeedback.setBatchTask((task) => task?.operationKey === operationKey ? { ...task, results } : task),
      onTask: (next) => { currentId = next.taskId; operationFeedback.setBatchTask((task) => task?.operationKey === operationKey ? { ...task, ...next } : task) },
      onCopyConflict: (conflict) => operationFeedback.requestConflict(conflict, ownerId),
    }
    operationFeedback.setBatchTask({ operationKey, ownerId, taskId, title: t('filebrowser.actions.copyTo', 'Copy to'), total: 0, results: [], running: true, reveal, cancel: () => { controller.abort(); operationFeedback.setBatchTask((task) => task ? { ...task, cancelling: true } : task) }, retry: () => {} })
    try {
      const results: OperationResult[] = retry ? await folderOps().copyEntries([], '', options) : await folderOps().resumeCopy!(taskId, options)
      operationFeedback.setBatchTask((task) => task ? { ...task, results, running: false, cancelling: false, retry: () => void view(currentId, true) } : task)
    } catch (error) {
      setError(toUiError(error).fallback)
      operationFeedback.setBatchTask((task) => task ? { ...task, running: false, cancelling: false } : task)
      setOpen(true)
    }
  }
  if (!folderOps().listCopies) return null
  return <>
    <button data-testid="copy-tasks" className="absolute bottom-1 right-3 z-20 rounded bg-[color:var(--cp-surface)] px-3 py-1 text-xs underline" onClick={() => { setOpen(true); void refresh() }}>{t('filebrowser.operation.copyTasks', 'Copy tasks')}</button>
    <Dialog open={open} onClose={() => setOpen(false)} fullWidth maxWidth="sm" data-testid="copy-tasks-dialog">
      <DialogTitle>{t('filebrowser.operation.copyTasks', 'Copy tasks')}</DialogTitle>
      <DialogContent>
        {error && <p role="alert">{error}</p>}
        {!tasks.length && <p>{t('filebrowser.operation.noCopyTasks', 'No submitted copy tasks')}</p>}
        {tasks.map((task) => <div key={task.task_id} className="border-b py-2"><p>{task.name} · {task.phase}</p><p className="break-all text-xs">{task.task_id}</p><Button disabled={operationFeedback.isRunning()} onClick={() => void view(task.task_id)}>{t('filebrowser.operation.viewTask', 'View task')}</Button></div>)}
      </DialogContent>
      <DialogActions><Button onClick={() => void refresh()}>{t('filebrowser.toolbar.refresh', 'Refresh')}</Button><Button onClick={() => setOpen(false)}>{t('common.close', 'Close')}</Button></DialogActions>
    </Dialog>
  </>
}
