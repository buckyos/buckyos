/**
 * Floating transfer list — renders the §4.7 TransferTask contract: stage +
 * determinate progress while running, retry/cancel context on failure, and
 * settled tasks stay dismissable until cleared.
 */

import { useSyncExternalStore } from 'react'
import { CheckCircle2, RotateCcw, Upload, X, XCircle } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import type { TransferStatus, TransferTask } from './data/state'
import { transferStore } from './data/transfers'
import { formatBytes } from './fileDisplay'

const STAGE_LABEL: Record<TransferStatus, { key: string; fallback: string }> = {
  queued: { key: 'filebrowser.transfer.stage.queued', fallback: 'Queued' },
  hashing: { key: 'filebrowser.transfer.stage.hashing', fallback: 'Hashing' },
  probing: { key: 'filebrowser.transfer.stage.probing', fallback: 'Checking destination' },
  uploading: { key: 'filebrowser.transfer.stage.uploading', fallback: 'Uploading' },
  committing: { key: 'filebrowser.transfer.stage.committing', fallback: 'Committing' },
  success: { key: 'filebrowser.transfer.stage.success', fallback: 'Done' },
  error: { key: 'filebrowser.transfer.stage.error', fallback: 'Failed' },
  cancelled: { key: 'filebrowser.transfer.stage.cancelled', fallback: 'Cancelled' },
}

const RUNNING: TransferStatus[] = ['queued', 'hashing', 'probing', 'uploading', 'committing']

function TaskRow({ task }: { task: TransferTask }) {
  const { t } = useI18n()
  const stage = STAGE_LABEL[task.status] ?? STAGE_LABEL.queued
  const running = RUNNING.includes(task.status)
  const percent =
    task.totalBytes > 0 ? Math.round((task.bytesSent / task.totalBytes) * 100) : 0

  return (
    <div className="px-3 py-2" data-testid={`transfer-${task.status}`}>
      <div className="flex items-center gap-2">
        {task.status === 'success' ? (
          <CheckCircle2 size={14} className="shrink-0 text-[color:var(--cp-success)]" />
        ) : task.status === 'error' ? (
          <XCircle size={14} className="shrink-0 text-[color:var(--cp-warning)]" />
        ) : (
          <Upload size={14} className="shrink-0 text-[color:var(--cp-accent)]" />
        )}
        <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-[color:var(--cp-text)]">
          {task.candidate.name}
        </span>
        <span className="shrink-0 text-[10px] uppercase tracking-wider text-[color:var(--cp-muted)]">
          {t(stage.key, stage.fallback)}
        </span>
        {task.status === 'error' || task.status === 'cancelled' ? (
          task.error?.retryable !== false ? (
            <button
              type="button"
              onClick={() => transferStore.retry(task.id)}
              aria-label={t('filebrowser.retry', 'Retry')}
              className="shrink-0 rounded-full p-1 text-[color:var(--cp-muted)] hover:text-[color:var(--cp-accent)]"
            >
              <RotateCcw size={12} />
            </button>
          ) : null
        ) : null}
        {running ? (
          <button
            type="button"
            onClick={() => transferStore.cancel(task.id)}
            aria-label={t('common.cancel', 'Cancel')}
            className="shrink-0 rounded-full p-1 text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]"
          >
            <X size={12} />
          </button>
        ) : (
          <button
            type="button"
            onClick={() => transferStore.dismiss(task.id)}
            aria-label={t('common.close', 'Close')}
            className="shrink-0 rounded-full p-1 text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]"
          >
            <X size={12} />
          </button>
        )}
      </div>
      {running ? (
        <div className="mt-1.5 flex items-center gap-2">
          <div className="h-1 flex-1 overflow-hidden rounded-full bg-[color:color-mix(in_srgb,var(--cp-border)_55%,transparent)]">
            <div
              className="h-full rounded-full transition-all"
              style={{ width: `${percent}%`, background: 'var(--cp-accent)' }}
            />
          </div>
          <span className="shrink-0 text-[10px] text-[color:var(--cp-muted)]">
            {formatBytes(task.bytesSent)} / {formatBytes(task.totalBytes)}
          </span>
        </div>
      ) : null}
      {task.status === 'error' && task.error ? (
        <p className="mt-1 text-[11px] text-[color:var(--cp-warning)]">
          {t(task.error.messageKey, task.error.fallback)}
        </p>
      ) : null}
    </div>
  )
}

export function TransfersPanel() {
  const { t } = useI18n()
  useSyncExternalStore(transferStore.subscribe, transferStore.snapshot)
  const tasks = transferStore.tasks()
  if (!tasks.length) return null

  const active = tasks.filter((task) => RUNNING.includes(task.status)).length

  return (
    <div
      className="absolute bottom-10 right-4 z-30 w-[300px] overflow-hidden rounded-[18px] border border-[color:var(--cp-border)] shadow-[0_14px_40px_rgba(0,0,0,0.24)]"
      style={{ background: 'var(--cp-surface)' }}
      data-testid="transfers-panel"
    >
      <div className="flex items-center gap-2 border-b border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] px-3 py-2">
        <span className="shell-kicker">
          {t('filebrowser.transfer.title', 'Transfers')}
        </span>
        <span className="ml-auto text-[10px] text-[color:var(--cp-muted)]">
          {active
            ? t('filebrowser.transfer.active', '{{count}} active', { count: active })
            : t('filebrowser.transfer.idle', 'idle')}
        </span>
      </div>
      <div className="max-h-[40vh] divide-y divide-[color:color-mix(in_srgb,var(--cp-border)_40%,transparent)] overflow-y-auto">
        {tasks.map((task) => (
          <TaskRow key={task.id} task={task} />
        ))}
      </div>
    </div>
  )
}
