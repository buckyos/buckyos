/* Cell bodies: what a `buckyos.cell` shows for each `view.type` (design §3.5.1), plus the write-lock bar. */

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { ServiceFailure } from '../api/client'
import { describeError, type ReadOk } from '../api/session'
import type { AssetContent, CellPayload, EntityEnvelope, Json, KeyedContent, RecordContent, RecordPropDef, Reference } from '../api/types'
import type { RichTextCollab } from '../richtext/collab'
import { RichTextEditor, StaticRichText } from '../richtext/RichTextEditor'
import { EDIT_STATE_LABEL } from '../state/edits'
import { useDirectReadOnly, useEdit, useLoad, useLocksVersion, useStore, useVersion, useWorkspaceUi } from '../state/hooks'
import { ConflictBox, TableViewCell } from './TableViewCell'
import { ValueEditor } from './ValueEditor'
import { UNSET, formatValue, type Input } from './values'

const MAX_EMBED_DEPTH = 3

// ---- write locks (design §2.11)

interface WriteAccess {
  /** The entity may be edited right now (capability present and, if required, the lock is held). */
  editable: boolean
  held: boolean
  required: boolean
}

function useWriteAccess(entity: EntityEnvelope | undefined, capability: 'update' | 'append' = 'update'): WriteAccess {
  const store = useStore()
  useLocksVersion()
  const readOnlyNow = useDirectReadOnly()
  if (!entity) return { editable: false, held: false, required: false }
  const capable = entity.capabilities.includes(capability) && !readOnlyNow
  const required = entity.write_policy === 'lock_required'
  const held = store.locks.isHeld(entity.entity_id)
  return { editable: capable && (!required || held), held, required }
}

/** Lock bar + focus scope: the lock is released a moment after focus leaves the cell, and on close. */
function LockScope({ entity, onAcquired, children }: { entity: EntityEnvelope | undefined; onAcquired?: () => void; children: ReactNode }) {
  const store = useStore()
  useLocksVersion()
  const releaseTimer = useRef<number | null>(null)
  const entityId = entity?.entity_id
  const required = entity?.write_policy === 'lock_required'
  useEffect(() => () => {
    if (releaseTimer.current !== null) window.clearTimeout(releaseTimer.current)
    if (entityId) void store.locks.release(entityId)
  }, [store, entityId])
  if (!entity || !required) return <>{children}</>
  const held = store.locks.isHeld(entity.entity_id)
  const holder = entity.lock_holder
  const lost = store.locks.lostReason(entity.entity_id)
  const canWrite = entity.capabilities.some((capability) => capability === 'update' || capability === 'append' || capability === 'delete' || capability === 'structure')
  const acquire = async () => {
    try {
      await store.locks.acquire(entity.entity_id)
      store.versions.bump(['outline'])
      onAcquired?.()
    } catch (error) {
      if (error instanceof ServiceFailure && error.code === 'LOCK_HELD') {
        const lock = ((error.data?.locks ?? []) as { holder?: string; expires_at?: string }[])[0]
        store.notify('info', `写锁由 ${lock?.holder ?? '他人'} 持有${lock?.expires_at ? `（租约至 ${new Date(lock.expires_at).toLocaleTimeString()}）` : ''}，当前只读。`)
        store.versions.bump(['outline'])
      } else {
        store.notify('error', `无法取得写锁：${describeError(error)}`)
      }
    }
  }
  const release = () => { void store.locks.release(entity.entity_id).then(() => store.versions.bump(['outline'])) }
  return (
    <div
      className="aiws-lock-scope"
      onFocus={() => { if (releaseTimer.current !== null) { window.clearTimeout(releaseTimer.current); releaseTimer.current = null } }}
      onBlur={(event) => {
        if (!held || event.currentTarget.contains(event.relatedTarget as Node | null)) return
        // Leaving the cell for a while ends the editing session; coming back cancels it.
        releaseTimer.current = window.setTimeout(release, 30_000)
      }}
    >
      <div className="aiws-lock-bar" data-testid={`aiws-lock-${entity.entity_id}`} data-held={held ? 'true' : 'false'}>
        {held ? (
          <><span>🔒 你持有写锁，正在编辑（每 20 秒续约）</span><button type="button" data-testid="aiws-lock-release" onClick={release}>结束编辑</button></>
        ) : holder ? (
          <><span data-testid="aiws-lock-holder">🔒 由 {holder.principal} 编辑中{holder.expires_at ? `（租约至 ${new Date(holder.expires_at).toLocaleTimeString()}）` : ''}，当前只读</span>
            {canWrite && <button type="button" onClick={() => { void acquire() }}>再试一次</button>}</>
        ) : (
          <><span>🔒 此对象启用了写锁：同一时间只有一人可以修改</span>
            {canWrite && <button type="button" data-testid="aiws-lock-acquire" onClick={() => { void acquire() }}>开始编辑</button>}</>
        )}
        {lost && !held && <span className="aiws-error" role="alert" data-testid="aiws-lock-lost">{lost}。未被接受的修改保留在“需要处理”中。</span>}
      </div>
      {children}
    </div>
  )
}

// ---- dispatcher

interface CellBodyProps { cellId: string; depth?: number }

export function CellBody({ cellId, depth = 0 }: CellBodyProps) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const version = useVersion(`e:${cellId}`)
  const load = useCallback(() => store.session.read<KeyedContent<CellPayload>>(cellId), [store, cellId])
  const cell = useLoad<ReadOk<KeyedContent<CellPayload>>>(load, version)
  const renderEmbed = useCallback((reference: Reference) => <EmbeddedCell reference={reference} depth={depth + 1} />, [depth])
  if (cell.error && !cell.data) return <div className="aiws-error" role="alert">无法读取单元：{cell.error}</div>
  if (!cell.data) return <div className="aiws-muted">载入中…</div>
  const payload = cell.data.content.payload
  const sourceId = payload.source_ref.entity_id
  const source = ui.byId.get(sourceId)
  const embedded = depth > 0
  switch (payload.view.type) {
    case 'table':
      return <TableCellHost cellId={cellId} source={source} embedded={embedded} />
    case 'richtext':
      return embedded
        ? <StaticRichText entityId={sourceId} renderEmbed={renderEmbed} />
        : <RichTextCell key={sourceId} entity={source} entityId={sourceId} renderEmbed={renderEmbed} />
    case 'record':
      return <LockScope entity={embedded ? undefined : source}><RecordCell entityId={sourceId} entity={source} readOnly={embedded} /></LockScope>
    case 'asset':
      return <AssetCell entityId={sourceId} fit={payload.options?.fit === 'cover' ? 'cover' : 'contain'} readOnly={embedded} />
    default:
      return <div className="aiws-warning">此版本不支持的视图类型：{String((payload.view as { type: string }).type)}</div>
  }
}

function TableCellHost({ cellId, source, embedded }: { cellId: string; source: EntityEnvelope | undefined; embedded: boolean }) {
  const ui = useWorkspaceUi()
  const access = useWriteAccess(source)
  const readOnlyNow = useDirectReadOnly()
  const lockBlocks = (access.required && !access.held) || readOnlyNow
  return (
    <LockScope entity={embedded ? undefined : source}>
      <TableViewCell cellId={cellId} readOnly={embedded || lockBlocks} compact={embedded} annotations={ui.annotations}
        onAnnotate={embedded ? undefined : ui.annotate ?? undefined} onActivateAnnotation={ui.setActiveAnnotation} />
    </LockScope>
  )
}

function EmbeddedCell({ reference, depth }: { reference: Reference; depth: number }) {
  const ui = useWorkspaceUi()
  const target = ui.byId.get(reference.entity_id)
  if (depth > MAX_EMBED_DEPTH) return <div className="aiws-muted">嵌入层级过深，未展开（{reference.entity_id}）</div>
  if (!target || target.deleted) return <div className="aiws-warning">嵌入的单元不存在或已删除（{reference.entity_id}）</div>
  if (target.type_id !== 'buckyos.cell') return <div className="aiws-warning">嵌入目标不是单元（{reference.entity_id}）</div>
  return (
    <div className="aiws-embedded" data-testid={`aiws-embed-${reference.entity_id}`}>
      <div className="aiws-embedded-title">嵌入 · {target.title ?? target.name ?? reference.entity_id}（只读）</div>
      <CellBody cellId={reference.entity_id} depth={depth} />
    </div>
  )
}

// ---- rich text

function RichTextCell({ entity, entityId, renderEmbed }: { entity: EntityEnvelope | undefined; entityId: string; renderEmbed: (reference: Reference) => ReactNode }) {
  const ui = useWorkspaceUi()
  const access = useWriteAccess(entity)
  const collabRef = useRef<RichTextCollab | null>(null)
  const onCollab = useCallback((collab: RichTextCollab | null) => { collabRef.current = collab }, [])
  const marks = useMemo(() => ui.annotations.filter((mark) => mark.payload.target.entity_id === entityId), [ui.annotations, entityId])
  return (
    <LockScope entity={entity} onAcquired={() => collabRef.current?.resume()}>
      <RichTextEditor
        entityId={entityId}
        editable={access.editable}
        entities={ui.entities}
        renderEmbed={renderEmbed}
        onOpenEntity={ui.openEntity}
        annotations={marks}
        activeAnnotation={ui.activeAnnotation}
        onActivateAnnotation={ui.setActiveAnnotation}
        onAnnotate={ui.annotate ?? undefined}
        onCollab={onCollab}
      />
    </LockScope>
  )
}

// ---- record (design §3.2)

function RecordCell({ entityId, entity, readOnly }: { entityId: string; entity: EntityEnvelope | undefined; readOnly: boolean }) {
  const store = useStore()
  const access = useWriteAccess(entity)
  const version = useVersion(`e:${entityId}`)
  const load = useCallback(() => store.session.read<RecordContent>(entityId), [store, entityId])
  const record = useLoad<ReadOk<RecordContent>>(load, version)
  if (record.error && !record.data) return <div className="aiws-error" role="alert">无法读取记录：{record.error}</div>
  if (!record.data) return <div className="aiws-muted">载入中…</div>
  const content = record.data.content
  return (
    <table className="aiws-record" data-testid={`aiws-record-${entityId}`}>
      <tbody>
        {content.schema.properties.map((prop) => (
          <RecordProp key={prop.key} entityId={entityId} prop={prop} value={content.props[prop.key]} rev={content.key_revs[`p:${prop.key}`] ?? 0} editable={!readOnly && access.editable} />
        ))}
      </tbody>
    </table>
  )
}

function RecordProp({ entityId, prop, value, rev, editable }: { entityId: string; prop: RecordPropDef; value: Json | undefined; rev: number; editable: boolean }) {
  const store = useStore()
  const editId = `key:${entityId}:p:${prop.key}`
  const entry = useEdit(editId)
  const [editing, setEditing] = useState<{ baseRev: number; text?: string } | null>(null)
  const pending = entry && entry.state !== 'committed' && entry.hasMine
  const write = (input: Input, baseRev: number) => store.submit({
    editId, label: `${prop.name}`, mine: input === UNSET ? null : input, hasMine: true,
    operations: [input === UNSET
      ? { op: 'entity.unset_keys', entity_id: entityId, keys: [{ key: `p:${prop.key}`, expect: { rev: baseRev } }] }
      : { op: 'entity.set_keys', entity_id: entityId, keys: [{ key: `p:${prop.key}`, value: input, expect: { rev: baseRev } }] }],
  })
  const shown = pending ? (entry.mine === null ? '' : formatValue(prop, entry.mine)) : formatValue(prop, value)
  return (
    <tr data-testid={`aiws-prop-${prop.key}`} data-state={entry?.state ?? 'clean'}>
      <th scope="row">{prop.name}{prop.required ? ' *' : ''}</th>
      <td>
        {editing ? (
          <ValueEditor def={prop} value={value} initialText={editing.text} ariaLabel={prop.name} onCancel={() => setEditing(null)}
            onCommit={(input) => { setEditing(null); void write(input, editing.baseRev) }} />
        ) : (
          <button type="button" className="aiws-cell-value" disabled={!editable} onClick={() => setEditing({ baseRev: rev })}>{shown || <span className="aiws-muted">（未设置）</span>}</button>
        )}
        {entry && !editing && <span className={`aiws-state aiws-state-${entry.state}`} data-testid="aiws-edit-state" title={entry.detail}>{EDIT_STATE_LABEL[entry.state]}</span>}
        {entry && entry.state === 'needs_attention' && !editing && (
          <ConflictBox
            store={store}
            editId={editId}
            theirs={entry.hasTheirs ? formatValue(prop, entry.theirs) || '（未设置）' : null}
            mine={entry.hasMine ? (entry.mine === null ? '（清空）' : formatValue(prop, entry.mine ?? undefined)) : null}
            detail={entry.detail ?? ''}
            onKeepMine={entry.hasMine && entry.currentRev !== undefined ? () => { void write(entry.mine === null ? UNSET : (entry.mine as Json), entry.currentRev ?? 0) } : undefined}
            onEditAgain={entry.hasMine ? () => setEditing({ baseRev: entry.currentRev ?? rev, text: entry.mine === null || typeof entry.mine === 'object' ? '' : String(entry.mine) }) : undefined}
          />
        )}
      </td>
    </tr>
  )
}

// ---- asset (design §3.6)

const AVAILABILITY_TEXT: Record<string, string> = { available: '可用', missing: '内容缺失（对象存储中取不到）', corrupt: '内容损坏（校验失败）' }

function AssetCell({ entityId, fit, readOnly }: { entityId: string; fit: 'contain' | 'cover'; readOnly: boolean }) {
  const store = useStore()
  const version = useVersion(`e:${entityId}`)
  const load = useCallback(() => store.session.read<AssetContent>(entityId), [store, entityId])
  const asset = useLoad<ReadOk<AssetContent>>(load, version)
  const objectId = asset.data?.content.payload.object_id
  const availability = asset.data?.content.availability
  const [image, setImage] = useState<{ objectId: string; url: string | null; error: string | null } | null>(null)
  const [uploading, setUploading] = useState<string | null>(null)
  useEffect(() => {
    if (!objectId || availability !== 'available') return
    let live = true
    let url: string | null = null
    // The asset route needs the session token, so the bytes are fetched here and shown through a blob URL.
    store.session.fetchAsset(objectId).then(
      (blob) => { if (!live) return; url = URL.createObjectURL(blob); setImage({ objectId, url, error: null }) },
      (error: unknown) => { if (live) setImage({ objectId, url: null, error: describeError(error) }) },
    )
    return () => { live = false; if (url) URL.revokeObjectURL(url) }
  }, [store, objectId, availability])
  if (asset.error && !asset.data) return <div className="aiws-error" role="alert">无法读取资产：{asset.error}</div>
  if (!asset.data) return <div className="aiws-muted">载入中…</div>
  const payload = asset.data.content.payload
  const isImage = (payload.media_type ?? '').startsWith('image/')
  const current = image && image.objectId === objectId ? image : null
  const canReplace = !readOnly && asset.data.capabilities.includes('update')
  const replace = async (file: File) => {
    setUploading('上传中…')
    try {
      const uploaded = await store.session.uploadAsset(file, file.name)
      const revs = asset.data?.content.key_revs ?? {}
      const outcome = await store.submit({
        editId: `key:${entityId}:object_id`, label: `替换资产 ${payload.file_name ?? entityId}`,
        operations: [{ op: 'entity.set_keys', entity_id: entityId, keys: [
          { key: 'object_id', value: uploaded.object_id, expect: { rev: revs.object_id ?? 0 } },
          { key: 'file_name', value: file.name, expect: { rev: revs.file_name ?? 0 } },
        ] }],
      })
      setUploading(outcome.status === 'accepted' ? null : '替换未被接受，见“需要处理”')
    } catch (error) {
      setUploading(`上传失败：${describeError(error)}`)
    }
  }
  return (
    <div className="aiws-asset" data-testid={`aiws-asset-${entityId}`} data-availability={availability}>
      {availability !== 'available' ? (
        <div className="aiws-asset-placeholder">资产不可用：{AVAILABILITY_TEXT[availability ?? ''] ?? availability}</div>
      ) : !isImage ? (
        <div className="aiws-asset-placeholder">附件 {payload.file_name ?? ''}（{payload.media_type ?? '未知类型'}）</div>
      ) : current?.url ? (
        <img src={current.url} alt={payload.file_name ?? entityId} style={{ objectFit: fit }} data-testid="aiws-asset-image" />
      ) : current?.error ? (
        <div className="aiws-error" role="alert">图片读取失败：{current.error}</div>
      ) : <div className="aiws-muted">读取图片…</div>}
      <div className="aiws-asset-meta">
        <span>{payload.file_name ?? entityId}</span>
        <span>{payload.media_type}</span>
        {typeof payload.size === 'number' && <span>{payload.size} 字节</span>}
        <span className={`aiws-chip ${availability === 'available' ? '' : 'aiws-chip-warn'}`} data-testid="aiws-asset-availability">{AVAILABILITY_TEXT[availability ?? ''] ?? availability}</span>
        {canReplace && (
          <label className="aiws-link">替换…
            <input type="file" hidden accept="image/*" onChange={(event) => { const file = event.target.files?.[0]; event.target.value = ''; if (file) void replace(file) }} />
          </label>
        )}
        {uploading && <span>{uploading}</span>}
      </div>
    </div>
  )
}
