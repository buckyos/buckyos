import { Button, Checkbox, Dialog, DialogActions, DialogContent, DialogTitle, FormControlLabel } from '@mui/material'
import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import type { FileItem } from '../data/FolderReader'
import type { ConflictChoice, OperationResult } from '../data/folderOps'
import type { ConflictRequest, BatchTask } from '../data/operationFeedback'
import { formatBytes, formatDate } from '../fileDisplay'

export interface DeleteRequest {
  items: FileItem[]
  location: string
  references: boolean
  submit: () => void
}
export function DeleteDialog({ request, onClose }: { request: DeleteRequest | null; onClose: () => void }) {
  const { t } = useI18n()
  if (!request) return null
  const title = request.references ? t('filebrowser.actions.removeFromCollection', 'Remove from collection') : t('filebrowser.operation.permanentDelete', 'Permanently delete')
  return <Dialog open onClose={onClose} maxWidth="sm" fullWidth data-testid="delete-dialog">
    <DialogTitle>{title} · {request.items.length}</DialogTitle>
    <DialogContent>
      <p className="break-all text-sm">{request.location}</p>
      <ul className="my-3 max-h-48 overflow-auto">{request.items.map((item) => <li className="break-all py-1" key={item.key}>{item.entry.name}</li>)}</ul>
      <p>{request.references ? t('filebrowser.operation.refsRemain', 'Original files remain in their folders.') : t('filebrowser.operation.cannotUndo', 'This cannot be undone. There is no recycle bin on this server.')}</p>
      {!request.references && request.items.some((item) => item.entry.kind === 'folder') && <p className="mt-2 font-semibold">{t('filebrowser.operation.folderContents', 'All contents of the selected folders will also be permanently deleted.')}</p>}
    </DialogContent>
    <DialogActions><Button onClick={onClose}>{t('common.cancel', 'Cancel')}</Button><Button color="error" onClick={() => { onClose(); request.submit() }}>{title}</Button></DialogActions>
  </Dialog>
}
export function ConflictDialog({ request }: { request: ConflictRequest | null }) {
  const { t, locale } = useI18n()
  const [apply, setApply] = useState(false)
  if (!request) return null
  const choose = (choice: ConflictChoice) => { request.resolve(choice, apply); setApply(false) }
  return <Dialog open onClose={() => choose('cancel')} maxWidth="sm" fullWidth data-testid="conflict-dialog">
    <DialogTitle>{t('filebrowser.operation.conflictTitle', 'A name already exists')}</DialogTitle>
    <DialogContent>
      <p className="break-all">{request.targetPath}</p>
      <table className="my-4 w-full text-left text-sm"><thead><tr><th>{t('filebrowser.operation.source', 'Source')}</th><th>{t('filebrowser.operation.destination', 'Destination')}</th></tr></thead><tbody>
        <tr>{[request.source, request.target].map((entry, i) => <td key={i} className="max-w-48 break-all p-2 align-top"><strong>{entry.name}</strong><p>{t(`filebrowser.kind.${entry.kind}`, entry.kind)}</p><p>{entry.kind === 'folder' ? t('filebrowser.meta.notCalculated', 'Not calculated') : formatBytes(entry.sizeBytes, locale)}</p><p>{formatDate(entry.modifiedAt, locale)}</p></td>)}</tr>
      </tbody></table>
      {(request.source.kind === 'folder' || request.target.kind === 'folder') && <p>{t('filebrowser.operation.noMerge', 'Folders will be kept separately. Their contents will not be merged.')}</p>}
      <FormControlLabel control={<Checkbox checked={apply} onChange={(_, checked) => setApply(checked)} />} label={t('filebrowser.operation.applyConflicts', 'Apply to remaining conflicts of this type')} />
    </DialogContent>
    <DialogActions className="flex-wrap"><Button onClick={() => choose('cancel')}>{t('filebrowser.operation.cancelRemaining', 'Cancel remaining')}</Button><Button onClick={() => choose('skip')}>{t('filebrowser.operation.skip', 'Skip')}</Button><Button variant="contained" onClick={() => choose('keep-both')}>{t('filebrowser.operation.keepBoth', 'Keep both')}</Button></DialogActions>
  </Dialog>
}
export function BatchResults({ task, onClose }: { task: BatchTask | null; onClose: () => void }) {
  const { t } = useI18n()
  const [expanded, setExpanded] = useState(true)
  if (!task) return null
  const count = (status: OperationResult['status']) => task.results.filter((result) => result.status === status).length
  const failed = count('failed')
  return <div data-testid="batch-results" className="absolute bottom-10 left-3 z-30 max-h-[55%] w-[min(430px,calc(100%-24px))] overflow-auto rounded-xl border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] p-3 text-xs shadow-xl" aria-live="polite">
    <div className="flex items-center gap-2"><button className="min-h-8 flex-1 text-left font-semibold" onClick={() => setExpanded(!expanded)}>{task.title} · {task.results.length}/{task.total}</button><button onClick={task.running ? task.cancel : onClose} className="min-h-8 px-2">{task.running ? t('filebrowser.operation.cancelRemaining', 'Cancel remaining') : t('common.close', 'Close')}</button></div>
    <p>{t('filebrowser.operation.counts', '{{success}} succeeded · {{failed}} failed · {{skipped}} skipped · {{cancelled}} cancelled', { success: count('success'), failed, skipped: count('skipped'), cancelled: count('cancelled') })}</p>
    {expanded && <ul className="mt-2 max-h-48 overflow-auto">{task.results.map((result, i) => <li key={i} className="break-all border-t border-[color:var(--cp-border)] py-2" data-status={result.status}><strong>{result.entry.name}</strong> · {t(`filebrowser.operation.${result.status}`, result.status)}<p>{result.entry.path}{result.targetPath ? ` → ${result.targetPath}` : ''}</p>{result.error && <p>{t(result.error.messageKey, result.error.fallback)}</p>}</li>)}</ul>}
    {!task.running && failed > 0 && <Button size="small" onClick={task.retry}>{t('filebrowser.operation.retryFailed', 'Retry failed items')}</Button>}
  </div>
}
