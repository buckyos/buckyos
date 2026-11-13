import { useCallback, useRef, useState } from 'react'
import { ContentPreview } from '../../../../components/ContentPreview'
import { describeError, type ReadOk } from '../../api/session'
import type { EntityEnvelope, KeyedContent, Operation } from '../../api/types'
import { useLoad, useOutlineVersion, useStore, useVersion, useWorkspaceUi, useWriteAccess } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { LockScope } from '../blocks/editors'
import { assetSource } from '../media/source'
import { saveBlob } from '../shell/download'
import { useFilePicker } from '../shell/filePicker'
import { entityLabel } from './dataOps'

type FileMember = { kind: 'file'; object_id: string; media_type?: string; size?: number }
type DirectoryMember = FileMember | { kind: 'dir'; entity_id: string }
type DirectoryContent = KeyedContent<{ members: Record<string, DirectoryMember> }>
type Rename = { from: string; to: string; member: DirectoryMember; rev: number }

export function DirectoryDetail({ entity, readOnly }: { entity: EntityEnvelope; readOnly: boolean }) {
  const { t, compare } = useAiwsText()
  const store = useStore()
  const ui = useWorkspaceUi()
  const access = useWriteAccess(entity)
  useOutlineVersion()
  const version = useVersion(`e:${entity.entity_id}`)
  const load = useCallback(() => store.session.read<DirectoryContent>(entity.entity_id), [store, entity.entity_id])
  const read = useLoad<ReadOk<DirectoryContent>>(load, version)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const pending = useRef(false)
  const [rename, setRename] = useState<Rename | null>(null)
  const [preview, setPreview] = useState<string | null>(null)
  const [directoryId, setDirectoryId] = useState('')
  const [directoryName, setDirectoryName] = useState('')
  const picker = useFilePicker(setError)
  const current = read.data?.entity_id === entity.entity_id ? read.data : undefined
  const members = current?.content.payload.members ?? {}
  const revisions = current?.content.key_revs ?? {}
  const mayEdit = !readOnly && !entity.source && !entity.degraded && entity.capabilities.includes('update')
  const editable = mayEdit && access.editable && !busy && Boolean(current) && !read.error
  const directories = store.outline.all().filter((item) => item.type_id === 'buckyos.dir' && !item.deleted && !item.degraded && item.entity_id !== entity.entity_id && item.capabilities.includes('read'))
  const entries = Object.entries(members).sort(([a, x], [b, y]) => (x.kind === y.kind ? compare(a, b) : x.kind === 'dir' ? -1 : 1))
  const previewMember = preview ? members[preview] : undefined

  const validName = (name: string) => {
    if (!name || new TextEncoder().encode(name).length > 255 || name.includes('\0') || name.split('/').some((part) => !part || part === '.' || part === '..')) {
      throw new Error(t('aiworkspace.directory.invalidName'))
    }
  }
  const unusedName = (name: string) => {
    validName(name)
    if (Object.hasOwn(members, name)) throw new Error(t('aiworkspace.directory.nameExists', { name }))
  }
  const run = async (action: () => Promise<void>) => {
    if (pending.current) return
    pending.current = true
    setBusy(true)
    setError(null)
    try { await action() } catch (failure) { setError(describeError(failure)) }
    finally { pending.current = false; setBusy(false) }
  }
  const submit = async (label: string, operations: Operation[]) => {
    const editId = `directory:${entity.entity_id}`
    const outcome = await store.submit({ editId, label, operations })
    if (outcome.status !== 'accepted' && outcome.status !== 'saved_locally') {
      throw new Error(store.edits.get(editId)?.detail ?? t('aiworkspace.directory.saveFailed'))
    }
    read.reload()
  }
  const upload = () => run(async () => {
    if (!editable) return
    const files = await picker.files({ multiple: true })
    if (!files?.length) return
    const names = new Set<string>()
    for (const file of files) {
      unusedName(file.name)
      if (names.has(file.name)) throw new Error(t('aiworkspace.directory.nameExists', { name: file.name }))
      names.add(file.name)
    }
    const operations: Operation[] = []
    for (const file of files) {
      const asset = await store.session.uploadAsset(file, file.name)
      operations.push({ op: 'dir.set_member', entity_id: entity.entity_id, name: file.name, member: { kind: 'file', object_id: asset.object_id }, expect: { rev: revisions[`m:${file.name}`] ?? 0 } })
    }
    await submit(t('aiworkspace.directory.uploadEdit', { count: files.length }), operations)
  })
  const replace = (name: string) => run(async () => {
    if (!editable) return
    const file = await picker.file()
    if (!file) return
    const asset = await store.session.uploadAsset(file, file.name)
    await submit(t('aiworkspace.directory.replaceEdit', { name }), [
      { op: 'dir.set_member', entity_id: entity.entity_id, name, member: { kind: 'file', object_id: asset.object_id }, expect: { rev: revisions[`m:${name}`] ?? 0 } },
    ])
  })
  const saveRename = () => run(async () => {
    if (!editable || !rename) return
    if (rename.to === rename.from) { setRename(null); return }
    unusedName(rename.to)
    await submit(t('aiworkspace.directory.renameEdit', { from: rename.from, to: rename.to }), [
      { op: 'dir.remove_member', entity_id: entity.entity_id, name: rename.from, expect: { rev: rename.rev } },
      { op: 'dir.set_member', entity_id: entity.entity_id, name: rename.to, member: rename.member, expect: { rev: revisions[`m:${rename.to}`] ?? 0 } },
    ])
    if (preview === rename.from) setPreview(rename.to)
    setRename(null)
  })
  const remove = (name: string) => run(async () => {
    if (!editable) return
    await submit(t('aiworkspace.directory.removeEdit', { name }), [
      { op: 'dir.remove_member', entity_id: entity.entity_id, name, expect: { rev: revisions[`m:${name}`] ?? 0 } },
    ])
    if (preview === name) setPreview(null)
    if (rename?.from === name) setRename(null)
  })
  const addDirectory = () => run(async () => {
    if (!editable || !directoryId) return
    unusedName(directoryName)
    await submit(t('aiworkspace.directory.addEdit', { name: directoryName }), [
      { op: 'dir.set_member', entity_id: entity.entity_id, name: directoryName, member: { kind: 'dir', entity_id: directoryId }, expect: { rev: revisions[`m:${directoryName}`] ?? 0 } },
    ])
    setDirectoryId('')
    setDirectoryName('')
  })
  const download = (name: string, member: FileMember) => run(async () => {
    saveBlob(await store.session.fetchAsset(member.object_id), name.split('/').at(-1) ?? name)
  })

  return (
    <LockScope entity={mayEdit ? entity : undefined}>
      <section className="aiws-directory" data-testid="aiws-directory" aria-busy={busy}>
        <div className="aiws-inline-form">
          <b>{t('aiworkspace.directory.title')}</b>
          {current && <span className="aiws-muted">{t('aiworkspace.directory.count', { count: entries.length })}</span>}
          {mayEdit && <button type="button" data-testid="aiws-directory-upload" disabled={!editable} onClick={() => { void upload() }}>{t('aiworkspace.directory.upload')}</button>}
          {busy && <span role="status" className="aiws-muted">{t('aiworkspace.directory.working')}</span>}
        </div>
        <p className="aiws-muted">{t('aiworkspace.directory.description')}</p>
        {!mayEdit && <div className="aiws-muted" data-testid="aiws-directory-readonly">{t('aiworkspace.directory.readOnly')}</div>}
        {(error || read.error) && <div className="aiws-error" role="alert" data-testid="aiws-directory-error">{error || read.error} {read.error && <button type="button" onClick={read.reload}>{t('aiworkspace.common.retry')}</button>}</div>}
        {!current && !read.error && <div className="aiws-muted">{t('aiworkspace.directory.loading')}</div>}
        {current && entries.length === 0 && <div className="aiws-directory-empty aiws-muted" data-testid="aiws-directory-empty">{t('aiworkspace.directory.empty')}</div>}
        {entries.length > 0 && <div className="aiws-directory-scroll"><table className="aiws-record aiws-directory-table">
          <thead><tr><th scope="col">{t('aiworkspace.directory.name')}</th><th scope="col">{t('aiworkspace.directory.type')}</th><th scope="col">{t('aiworkspace.directory.size')}</th><th scope="col">{t('aiworkspace.directory.actions')}</th></tr></thead>
          <tbody>{entries.map(([name, member]) => (
            <tr key={name} data-testid="aiws-directory-member" data-name={name} data-kind={member.kind}>
              <td>{rename?.from === name ? <form className="aiws-inline-form" onSubmit={(event) => { event.preventDefault(); void saveRename() }}>
                <input data-testid="aiws-directory-name" aria-label={t('aiworkspace.directory.newName')} autoFocus value={rename.to} disabled={!editable} onChange={(event) => setRename({ ...rename, to: event.target.value })} />
                <button type="submit" data-testid="aiws-directory-save" disabled={!editable}>{t('aiworkspace.common.save')}</button>
                <button type="button" disabled={busy} onClick={() => setRename(null)}>{t('aiworkspace.common.cancel')}</button>
              </form> : <button type="button" className="aiws-link aiws-directory-name" data-testid="aiws-directory-open" onClick={() => member.kind === 'dir' ? ui.openEntity(member.entity_id) : setPreview(name)}>{name}</button>}</td>
              <td>{member.kind === 'dir' ? t('aiworkspace.directory.directory') : member.media_type || t('aiworkspace.directory.file')}</td>
              <td>{member.kind === 'file' && typeof member.size === 'number' ? t('aiworkspace.directory.bytes', { count: member.size }) : '—'}</td>
              <td><div className="aiws-inline-form">
                {member.kind === 'file' && entity.capabilities.includes('export') && <button type="button" data-testid="aiws-directory-download" disabled={busy} onClick={() => { void download(name, member) }}>{t('aiworkspace.directory.download')}</button>}
                {mayEdit && <>
                  {member.kind === 'file' && <button type="button" data-testid="aiws-directory-replace" disabled={!editable} onClick={() => { void replace(name) }}>{t('aiworkspace.directory.replace')}</button>}
                  <button type="button" data-testid="aiws-directory-rename" disabled={!editable} onClick={() => { setError(null); setRename({ from: name, to: name, member, rev: revisions[`m:${name}`] ?? 0 }) }}>{t('aiworkspace.directory.rename')}</button>
                  <button type="button" data-testid="aiws-directory-remove" disabled={!editable} onClick={() => { void remove(name) }}>{t('aiworkspace.directory.remove')}</button>
                </>}
              </div></td>
            </tr>
          ))}</tbody>
        </table></div>}
        {mayEdit && directories.length > 0 && <form className="aiws-inline-form" data-testid="aiws-directory-add-dir" onSubmit={(event) => { event.preventDefault(); void addDirectory() }}>
          <label>{t('aiworkspace.directory.addDirectory')}
            <select data-testid="aiws-directory-dir-target" aria-label={t('aiworkspace.directory.selectDirectory')} disabled={!editable} value={directoryId} onChange={(event) => {
              setDirectoryId(event.target.value)
              const target = store.outline.get(event.target.value)
              if (target) setDirectoryName(entityLabel(target))
            }}><option value="">{t('aiworkspace.directory.selectDirectory')}</option>{directories.map((item) => <option value={item.entity_id} key={item.entity_id}>{entityLabel(item)}</option>)}</select>
          </label>
          <input data-testid="aiws-directory-dir-name" aria-label={t('aiworkspace.directory.memberName')} placeholder={t('aiworkspace.directory.memberName')} disabled={!editable} value={directoryName} onChange={(event) => setDirectoryName(event.target.value)} />
          <button type="submit" data-testid="aiws-directory-add" disabled={!editable || !directoryId || !directoryName}>{t('aiworkspace.directory.add')}</button>
        </form>}
        {preview && previewMember?.kind === 'file' && <div className="aiws-directory-preview aiws-preview-host" data-testid="aiws-directory-preview">
          <div className="aiws-inline-form"><b>{preview}</b><button type="button" onClick={() => setPreview(null)}>{t('aiworkspace.common.close')}</button></div>
          <ContentPreview key={`${preview}:${previewMember.object_id}`} source={assetSource(store, { ...previewMember, file_name: preview }, preview)} allowExport={entity.capabilities.includes('export')} uiMode="visible" onRequestExit={() => setPreview(null)} />
        </div>}
      </section>
    </LockScope>
  )
}
