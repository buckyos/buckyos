import { mockAttachmentUrl, readMockAttachment } from '../mock/attachments'
import { Download, FolderOpen } from 'lucide-react'
import { useState } from 'react'
import { homeStationTransport } from '../api/transport'

export function ObjectAttachment({ objId, name }: { objId: string; name: string }) {
  const transport = homeStationTransport()
  const [entries, setEntries] = useState<Record<string, string> | null>(null)
  const [error, setError] = useState('')
  const directory = objId.startsWith('cydir:')
  const read = (id: string, content = false) => transport ? fetch(content ? transport.contentUrl(id) : transport.objectUrl(id)) : readMockAttachment(id)
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
  const pickDirectory = (window as Window & { showDirectoryPicker?: () => Promise<FileSystemDirectoryHandle> }).showDirectoryPicker
  const saveDirectory = async () => {
    if (!pickDirectory) return
    try {
      const destination = await pickDirectory.call(window)
      const save = async (id: string, name: string, parent: FileSystemDirectoryHandle) => {
        const response = await read(id, !id.startsWith('cydir:'))
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        if (id.startsWith('cydir:')) {
          try { await parent.getDirectoryHandle(name); throw new Error('A folder with this name already exists; choose another destination.') } catch (error) { if (!(error instanceof DOMException) || error.name !== 'NotFoundError') throw error }
          const target = await parent.getDirectoryHandle(name, { create: true })
          const object = await response.json() as { body: Record<string, string> }
          for (const [name, child] of Object.entries(object.body)) await save(child, name, target)
        } else {
          const writable = await (await parent.getFileHandle(name, { create: true })).createWritable()
          try { await writable.write(await response.blob()); await writable.close() } catch (error) { await writable.abort(); throw error }
        }
      }
      await save(objId, name, destination)
      setError('')
    } catch (error) { setError(error instanceof Error ? error.message : String(error)) }
  }
  return <div className="my-1 rounded-lg border p-2" style={{ borderColor: 'var(--hs-divider)' }}>
    {directory ? <button type="button" onClick={() => void open()} className="flex items-center gap-2"><FolderOpen size={16} />{name}</button>
      : <a href={transport ? transport.contentUrl(objId) : mockAttachmentUrl(objId)} download={name} className="flex items-center gap-2"><Download size={16} />{name}</a>}
    {directory && pickDirectory ? <button type="button" className="text-xs underline" onClick={() => void saveDirectory()}>Save folder</button> : null}
    {error ? <p role="alert">{error}</p> : null}
    {entries ? <ul className="ml-3">{Object.entries(entries).map(([name, child]) => <li key={name}><ObjectAttachment objId={child} name={name} /></li>)}</ul> : null}
  </div>
}
