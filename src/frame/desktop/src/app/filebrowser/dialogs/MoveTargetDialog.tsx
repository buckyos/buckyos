import { Button, Dialog, DialogActions, DialogContent, DialogTitle } from '@mui/material'
import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { folderOps } from '../data/folderOps'
import { useFolderList } from '../data/useFolderList'
import { crumbsForUrl, dfsPathOf, normalizeUrl } from '../data/urls'
import { entryNameSchema } from '../data/schemas'
import { toUiError } from '../data/state'
import type { FileEntry } from '../types'
import { NamePromptDialog } from './NamePromptDialog'

export interface TargetRequest {
  entries: FileEntry[]
  initial: string
  references?: boolean
  copy?: boolean
  submit: (path: string, entries?: FileEntry[]) => void
}
export function MoveTargetDialog({ request, onClose }: { request: TargetRequest; onClose: () => void }) {
  const { t } = useI18n()
  const [url, setUrl] = useState(normalizeUrl(request.initial))
  const [draft, setDraft] = useState(request.initial)
  const [creating, setCreating] = useState(false)
  const [selected, setSelected] = useState<Map<string, FileEntry>>(new Map())
  const list = useFolderList(url, 'name', 'asc')
  const path = dfsPathOf(url)
  const invalid = !path || (!request.references && (!list.capabilities.acceptsContent || request.entries.some((entry) => entry.kind === 'folder' && (path === entry.path || path.startsWith(`${entry.path}/`)))))
  const navigate = (path: string) => { setUrl(normalizeUrl(path)); setDraft(path); setSelected(new Map()) }
  const recent: string[] = (() => { try { return JSON.parse(localStorage.getItem('files.moveTargets') ?? '[]') } catch { return [] } })()
  const submit = () => {
    if (invalid || !path || list.status !== 'ready') return
    localStorage.setItem('files.moveTargets', JSON.stringify([path, ...recent.filter((item) => item !== path)].slice(0, 5)))
    onClose()
    request.submit(path, [...selected.values()])
  }
  return <Dialog open onClose={onClose} maxWidth="sm" fullWidth data-testid="target-dialog">
    <DialogTitle>{request.references ? t('filebrowser.operation.addExisting', 'Add existing files') : request.copy ? t('filebrowser.actions.copyTo', 'Copy to') : t('filebrowser.actions.moveTo', 'Move to')} · {request.entries.length || selected.size}</DialogTitle>
    <DialogContent>
      {request.entries.length > 0 && <p className="mb-2 max-h-16 overflow-auto text-sm">{request.entries.map((entry) => entry.name).join(', ')}</p>}
      <form onSubmit={(event) => { event.preventDefault(); navigate(draft) }} className="flex gap-2"><input aria-label={t('filebrowser.operation.destination', 'Destination')} value={draft} onChange={(event) => setDraft(event.target.value)} className="min-w-0 flex-1 rounded border bg-transparent p-2" /><Button type="submit">{t('filebrowser.operation.go', 'Go')}</Button></form>
      <div className="my-2 flex flex-wrap">{crumbsForUrl(url).map((crumb) => <Button size="small" key={crumb.url} onClick={() => navigate(crumb.url)}>{crumb.label} /</Button>)}</div>
      {recent.length > 0 && <details className="my-2 text-sm"><summary>{t('filebrowser.operation.recentTargets', 'Recent destinations')}</summary>{recent.map((path) => <button className="block min-h-8 break-all" key={path} onClick={() => navigate(path)}>{path}</button>)}</details>}
      <div className="h-56 overflow-auto rounded border border-[color:var(--cp-border)]">
        {list.status === 'error' && <p role="alert">{list.error?.message}<Button onClick={() => list.reload()}>{t('filebrowser.retry', 'Retry')}</Button></p>}
        {list.loadedKeys().map((key) => list.loadedItemByKey(key)!).filter((item) => request.references || item.entry.kind === 'folder').map(({ entry }) => <div key={entry.id} className="flex min-h-11 items-center border-b border-[color:var(--cp-border)] px-2">
          {request.references && <input type="checkbox" aria-label={entry.name} checked={selected.has(entry.id)} onChange={() => setSelected((prev) => { const next = new Map(prev); if (next.has(entry.id)) next.delete(entry.id); else next.set(entry.id, entry); return next })} />}
          <button className="min-h-11 min-w-0 flex-1 truncate px-2 text-left" onClick={() => entry.kind === 'folder' ? navigate(entry.path) : setSelected(new Map([[entry.id, entry]]))}>{entry.kind === 'folder' ? '📁 ' : ''}{entry.name}</button>
        </div>)}
        {(list.hasMore || (list.totalCount ?? 0) > list.loadedKeys().length) && <Button onClick={() => list.ensureRange(list.loadedKeys().length, list.loadedKeys().length + 199)}>{t('filebrowser.operation.loadMore', 'Load more')}</Button>}
      </div>
      {invalid && <p role="alert" className="mt-2 text-sm">{t('filebrowser.operation.invalidDestination', 'Choose a writable folder outside the selected folders.')}</p>}
      {list.capabilities.acceptsContent && <Button onClick={() => setCreating(true)}>{t('filebrowser.operation.createHere', 'New folder here')}</Button>}
      <NamePromptDialog request={creating ? { title: t('filebrowser.actions.newFolder', 'New folder'), label: t('filebrowser.prompt.folderName', 'Folder name'), submitLabel: t('filebrowser.actions.create', 'Create'), schema: entryNameSchema, onSubmit: async (name) => { try { await folderOps().createFolder(path!, name) } catch (err) { throw new Error(toUiError(err).fallback) } } } : null} onClose={() => setCreating(false)} />
    </DialogContent>
    <DialogActions><Button onClick={onClose}>{t('common.cancel', 'Cancel')}</Button><Button disabled={invalid || list.status !== 'ready' || (request.references && !selected.size)} onClick={submit} variant="contained">{request.references ? t('filebrowser.operation.addReferences', 'Add references') : request.copy ? t('filebrowser.operation.copyHere', 'Copy here') : t('filebrowser.operation.moveHere', 'Move here')}</Button></DialogActions>
  </Dialog>
}
