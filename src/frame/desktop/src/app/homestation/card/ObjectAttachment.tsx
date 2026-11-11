import { mockAttachmentUrl, readMockAttachment } from '../mock/attachments'
import { Download, File, FolderCheck, FolderDown, FolderOpen, Loader2 } from 'lucide-react'
import { useState } from 'react'
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
    <button type="button" onClick={run} className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg" aria-label={label} title={label}
      aria-busy={state.phase === 'saving'} data-testid="hs-attachment-save" data-state={state.phase}
      style={{ color: state.phase === 'failed' ? 'var(--cp-danger)' : 'inherit' }}>
      {state.phase === 'saving' ? <Loader2 size={15} className="animate-spin" /> : state.phase === 'saved' ? <FolderCheck size={15} /> : <FolderDown size={15} />}
    </button>
  )
}

export function ObjectAttachment({ objId, name }: { objId: string; name: string }) {
  const { t } = useI18n()
  const transport = homeStationTransport()
  const [entries, setEntries] = useState<Record<string, string> | null>(null)
  const [error, setError] = useState('')
  // Downloading to this device is a desktop's extra next to saving into the Zone.
  const [canDownload] = useState(() => !window.matchMedia?.('(pointer: coarse)').matches)
  const directory = objId.startsWith('cydir:')
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
  return <div className="my-1 rounded-lg border p-2" style={{ borderColor: 'var(--hs-divider)' }} data-testid="hs-object-attachment" data-name={name}>
    <span className="flex items-center gap-1">
      {directory
        ? <button type="button" onClick={() => void open()} className="flex min-w-0 flex-1 items-center gap-2 text-left"><FolderOpen size={16} className="shrink-0" /><span className="truncate">{name}</span></button>
        : <span className="flex min-w-0 flex-1 items-center gap-2"><File size={16} className="shrink-0" /><span className="truncate">{name}</span></span>}
      <SaveButton objId={objId} name={name} />
      {!directory && canDownload
        ? <a href={transport ? transport.contentUrl(objId) : mockAttachmentUrl(objId)} download={name} className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg" aria-label={t('homestation.attachment.download', undefined, { name })} title={t('homestation.attachment.download', undefined, { name })}><Download size={15} /></a>
        : null}
    </span>
    {error ? <p role="alert">{error}</p> : null}
    {entries ? <ul className="ml-3">{Object.entries(entries).map(([name, child]) => <li key={name}><ObjectAttachment objId={child} name={name} /></li>)}</ul> : null}
  </div>
}
