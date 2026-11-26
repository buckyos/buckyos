import { mockAttachmentUrl, readMockAttachment } from '../mock/attachments'
import { Download, File, FolderCheck, FolderDown, FolderOpen, Loader2 } from 'lucide-react'
import { useState, type DragEvent } from 'react'
import { readZoneContent } from 'buckyos/content'
import { contentRefsRepresentations } from 'buckyos/transfer'
import { writeTransferEvent } from '../../../transfer'
import { useSaveToZone } from '../../../components/saveToZone'
import { isMockRuntime } from '../../../runtime'
import { useI18n } from '../../../i18n/provider'
import { homeStationTransport } from '../api/transport'

const read = (id: string, content = false) => {
  const transport = homeStationTransport()
  return transport ? fetch(content ? transport.contentUrl(id) : transport.objectUrl(id)) : readMockAttachment(id)
}

/** The bytes of a posted file, or of every file of a posted folder by its path below it (the mock runtime's save). */
async function postedFiles(objId: string): Promise<Array<{ path: string; blob: Blob }>> {
  const files: Array<{ path: string; blob: Blob }> = []
  const walk = async (id: string, path: string) => {
    const response = await read(id, !id.startsWith('cydir:'))
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
    if (!id.startsWith('cydir:')) { files.push({ path, blob: await response.blob() }); return }
    const object = await response.json() as { body: Record<string, string> }
    for (const [name, child] of Object.entries(object.body)) await walk(child, path ? `${path}/${name}` : name)
  }
  await walk(objId, '')
  return files
}

/** Saves a posted file or folder into the viewer's own files, at a place they pick in the Zone. */
function SaveButton({ objId, name }: { objId: string; name: string }) {
  const { t } = useI18n()
  const save = useSaveToZone(t('homestation.attachment.saveNameTaken'))
  const [state, setState] = useState<{ phase: 'idle' | 'saving' | 'failed' } | { phase: 'saved'; path: string }>({ phase: 'idle' })
  const run = () => {
    if (state.phase === 'saving') return
    setState({ phase: 'saving' })
    save({
      name,
      kind: objId.startsWith('cydir:') ? 'dir' : 'file',
      grant: async () => {
        const transport = homeStationTransport()
        if (!transport) {
          if (isMockRuntime()) return objId
          throw new Error('unavailable')
        }
        return (await transport.prepareSave(objId)).objId
      },
      mockFiles: () => postedFiles(objId),
    }).then(result => setState(result.status === 'saved' ? { phase: 'saved', path: result.path } : { phase: 'idle' }), () => setState({ phase: 'failed' }))
  }
  const label = state.phase === 'saved' ? t('homestation.attachment.saved', undefined, { path: state.path })
    : state.phase === 'failed' ? t('homestation.attachment.saveFailed')
      : t('homestation.attachment.save', undefined, { name })
  return (
    <button type="button" onClick={run} className="hs-icon-btn" style={{ minWidth: 32, height: 32, padding: 0, color: state.phase === 'failed' ? 'var(--hs-danger-ink)' : undefined }} aria-label={label} title={label}
      aria-busy={state.phase === 'saving'} data-testid="hs-attachment-save" data-state={state.phase}>
      {state.phase === 'saving' ? <Loader2 size={16} className="animate-spin" /> : state.phase === 'saved' ? <FolderCheck size={16} /> : <FolderDown size={16} />}
    </button>
  )
}

export function ObjectAttachment({ objId, name }: { objId: string; name: string }) {
  const { t } = useI18n()
  const transport = homeStationTransport()
  const [entries, setEntries] = useState<Record<string, string> | null>(null)
  const [error, setError] = useState('')
  // Downloading to this device is a desktop's extra next to saving into the Zone; so is dragging it out.
  const [canDownload] = useState(() => !window.matchMedia?.('(pointer: coarse)').matches)
  const directory = objId.startsWith('cydir:')
  // The posted file or folder as a Zone reference (`obj://<objId>`, 剪贴板规范 §2.2): the target reads it with its own session.
  const ref = canDownload ? readZoneContent({ origin: 'zone', kind: directory ? 'folder' : 'file', source: { kind: 'object-id', objectId: objId }, hints: { name } }) : undefined
  const dragOut = (event: DragEvent) => {
    if (!ref) return
    event.stopPropagation()
    writeTransferEvent(event.dataTransfer, contentRefsRepresentations([ref]))
    event.dataTransfer.effectAllowed = 'copy'
  }
  const open = async () => {
    if (entries) { setEntries(null); return }
    try {
      const response = await read(objId)
      if (!response.ok) throw new Error(`HTTP ${response.status}`)
      const object = await response.json() as { body: Record<string, string> }
      setEntries(object.body)
      setError('')
    } catch (error) { setError(error instanceof Error ? error.message : String(error)) }
  }
  return <div className="rounded-xl px-3 py-2 text-[14px]" style={{ border: '1px solid var(--hs-line-strong)' }} data-testid="hs-object-attachment" data-name={name} draggable={ref ? true : undefined} onDragStart={ref ? dragOut : undefined}>
    <span className="flex items-center gap-1">
      {directory
        ? <button type="button" onClick={() => void open()} className="flex min-w-0 flex-1 items-center gap-2 text-left"><FolderOpen size={16} className="shrink-0" /><span className="truncate">{name}</span></button>
        : <span className="flex min-w-0 flex-1 items-center gap-2"><File size={16} className="shrink-0" /><span className="truncate">{name}</span></span>}
      <SaveButton objId={objId} name={name} />
      {!directory && canDownload
        ? <a href={transport ? transport.contentUrl(objId) : mockAttachmentUrl(objId)} download={name} className="hs-icon-btn" style={{ minWidth: 32, height: 32, padding: 0 }} aria-label={t('homestation.attachment.download', undefined, { name })} title={t('homestation.attachment.download', undefined, { name })}><Download size={16} /></a>
        : null}
    </span>
    {error ? <p role="alert">{error}</p> : null}
    {entries ? <ul className="ml-4 mt-1 space-y-1">{Object.entries(entries).map(([name, child]) => <li key={name}><ObjectAttachment objId={child} name={name} /></li>)}</ul> : null}
  </div>
}
