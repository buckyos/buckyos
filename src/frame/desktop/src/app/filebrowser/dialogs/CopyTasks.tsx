import { Button, Dialog, DialogActions, DialogContent, DialogTitle } from '@mui/material'
import { ListTodo, Upload, XCircle } from 'lucide-react'
import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { formatBytes } from '../fileDisplay'
import { useTransferSummary } from '../useTransfers'
import { folderOps, type BatchOptions, type OperationResult } from '../data/folderOps'
import { operationFeedback } from '../data/operationFeedback'
import { toUiError } from '../data/state'

/**
 * Status-bar slot for copy tasks. While uploads run (or some failed) it shows
 * their progress and speed as text instead, and opens the transfers panel.
 */
export function CopyTasks({ ownerId, reveal, compact = false, onShowTransfers }: { ownerId?: string; reveal: (path: string) => void; compact?: boolean; onShowTransfers?: () => void }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const transfer = useTransferSummary()
  const percent = transfer.totalBytes > 0 ? Math.floor((transfer.bytesSent / transfer.totalBytes) * 100) : null
  const transferState = transfer.active ? 'active' : transfer.failed ? 'failed' : 'idle'
  const transferText = transferState === 'active'
    ? [
        t('filebrowser.transfer.statusActive', 'Transferring {{count}}', { count: transfer.active }),
        percent !== null ? `${percent}%` : null,
        transfer.bytesPerSecond !== null ? `${formatBytes(Math.round(transfer.bytesPerSecond))}/s` : null,
      ].filter(Boolean).join(' · ')
    : transferState === 'failed'
      ? t('filebrowser.transfer.statusFailed', 'Transfers failed: {{count}}', { count: transfer.failed })
      : null
  const [tasks, setTasks] = useState<{ task_id: string; name: string; phase: string }[]>([])
  const [error, setError] = useState<ReturnType<typeof toUiError> | null>(null)
  const refresh = async () => {
    try { setTasks(await folderOps().listCopies?.() ?? []); setError(null) } catch (error) { setError(toUiError(error)) }
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
      setError(toUiError(error))
      operationFeedback.setBatchTask((task) => task ? { ...task, running: false, cancelling: false } : task)
      setOpen(true)
    }
  }
  const showTransfers = transferText !== null && onShowTransfers !== undefined
  const label = showTransfers ? transferText : t('filebrowser.operation.copyTasks', 'Copy tasks')
  const onClick = () => {
    if (showTransfers) onShowTransfers()
    else { setOpen(true); void refresh() }
  }
  const tone = showTransfers && transferState === 'failed' ? 'text-[color:var(--files-warning-ink)]' : 'text-[color:var(--cp-muted)]'
  if (!showTransfers && !folderOps().listCopies) return null
  return <>
    {compact
      ? <button type="button" data-testid="copy-tasks" data-transfer-state={showTransfers ? transferState : 'idle'} aria-label={label} title={label} className={`flex shrink-0 items-center justify-center rounded-full tabular-nums ${tone} active:bg-[color:var(--files-pressed)]`} onClick={onClick}>
          {!showTransfers ? <ListTodo size={20} /> : transferState === 'failed' ? <XCircle size={20} /> : percent !== null ? <span className="inline-flex items-center gap-0.5 text-[12px] font-medium"><Upload size={14} />{percent}%</span> : <Upload size={20} />}
        </button>
      : <button type="button" data-testid="copy-tasks" data-transfer-state={showTransfers ? transferState : 'idle'} className={`inline-flex h-6 min-w-0 items-center gap-1 rounded-[6px] px-1.5 text-[12px] tabular-nums ${tone} hover:bg-[color:var(--files-hover)] hover:text-[color:var(--cp-text)]`} onClick={onClick}>
          {!showTransfers ? <ListTodo size={14} className="shrink-0" /> : transferState === 'failed' ? <XCircle size={14} className="shrink-0" /> : <Upload size={14} className="shrink-0" />}
          <span className="min-w-0 truncate">{label}</span>
        </button>}
    <Dialog open={open} onClose={() => setOpen(false)} fullWidth maxWidth="sm" data-testid="copy-tasks-dialog">
      <DialogTitle>{t('filebrowser.operation.copyTasks', 'Copy tasks')}</DialogTitle>
      <DialogContent>
        {error && <p role="alert">{t(error.messageKey, error.fallback)}</p>}
        {!tasks.length && <p>{t('filebrowser.operation.noCopyTasks', 'No submitted copy tasks')}</p>}
        {tasks.map((task) => <div key={task.task_id} className="border-b py-2"><p>{task.name} · {task.phase}</p><p className="break-all text-xs">{task.task_id}</p><Button disabled={operationFeedback.isRunning()} onClick={() => void view(task.task_id)}>{t('filebrowser.operation.viewTask', 'View task')}</Button></div>)}
      </DialogContent>
      <DialogActions><Button onClick={() => void refresh()}>{t('filebrowser.topbar.refresh', 'Refresh')}</Button><Button onClick={() => setOpen(false)}>{t('common.close', 'Close')}</Button></DialogActions>
    </Dialog>
  </>
}
