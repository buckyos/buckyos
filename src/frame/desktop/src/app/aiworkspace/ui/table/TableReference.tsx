import { useCallback, useContext, useRef, useState } from 'react'
import { ExternalLink, Upload } from 'lucide-react'
import { randomId } from '../../api/ids'
import { describeError } from '../../api/session'
import type { AssetContent, EntityEnvelope, Json } from '../../api/types'
import { useEntity, useLoad, useOutlineVersion, useStore, useVersion } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { AssetBlobImage } from '../extensions/AssetBlobImage'
import { stageMediaFile } from '../media/stage'
import { ShellContext } from '../shell/shellContext'
import { orderKeyAfter } from '../surfaces/tree'
import { referenceTypeLabels, type ValueDef } from '../values'

function readableTarget(entity: EntityEnvelope | undefined, field: ValueDef) {
  return entity && !entity.deleted && entity.capabilities.includes('read') && (!field.ref_purpose ? !field.target_types || field.target_types.includes(entity.type_id) : field.ref_purpose === 'image'
    ? entity.type_id === 'buckyos.asset-ref' && entity.media_type?.startsWith('image/')
    : entity.type_id === 'buckyos.container' && entity.kind === 'surface')
}

function refId(value: Json | undefined) {
  return value && typeof value === 'object' && !Array.isArray(value) && typeof value.entity_id === 'string' ? value.entity_id : ''
}

function ImageValue({ entity }: { entity: EntityEnvelope }) {
  const store = useStore()
  const { t } = useAiwsText()
  const version = useVersion(`e:${entity.entity_id}`)
  const load = useCallback(() => store.readBatched<AssetContent>(entity.entity_id), [store, entity.entity_id])
  const read = useLoad(load, version)
  if (read.error) return <span>{t('aiworkspace.table.ref.unavailable')}</span>
  const payload = read.data?.content.payload
  if (!payload) return null
  return <span className="aiws-cell-image"><AssetBlobImage key={payload.object_id} objectId={payload.object_id} mediaType={entity.media_type!} alt={payload.file_name ?? entity.title ?? ''} /></span>
}

export function ReferenceDisplay({ field, value, print }: { field: ValueDef; value: Json | undefined; print?: boolean }) {
  const { t } = useAiwsText()
  const shell = useContext(ShellContext)
  const id = refId(value)
  const entity = useEntity(id)
  if (!id) return null
  if (!entity || !readableTarget(entity, field)) return <span className="aiws-muted">{t('aiworkspace.table.ref.unavailable')}</span>
  if (field.ref_purpose === 'image') return <ImageValue key={id} entity={entity} />
  return <span className="aiws-cell-reference">{entity.title ?? entity.name ?? id}
    {!print && shell && field.ref_purpose === 'canvas' && <button type="button" aria-label={t('aiworkspace.table.ref.enter')} title={t('aiworkspace.table.ref.enter')}
      onPointerDown={(event) => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); shell.selectSurface(id); shell.setTopMode('canvas') }}><ExternalLink size={14} /></button>}
  </span>
}

export function ReferenceInput({ field, value, onChange, onCancel, disabled, autoFocus, ariaLabel }: {
  field: ValueDef; value: Json | undefined; onChange: (value: Json | undefined) => void; onCancel?: () => void
  disabled?: boolean; autoFocus?: boolean; ariaLabel?: string
}) {
  const store = useStore()
  const { t } = useAiwsText()
  const shell = useContext(ShellContext)
  useOutlineVersion()
  const file = useRef<HTMLInputElement>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const parent = (shell?.topMode === 'canvas' && shell.activeSurface?.content_folder_id) || 'data'
  const current = refId(value)
  const choices = store.outline.all().filter((e) => readableTarget(e, field) && (field.ref_purpose !== 'canvas' || e.entity_id !== shell?.activeSurface?.entity_id || shell?.topMode !== 'canvas'))
  const labelCounts = new Map<string, number>()
  for (const e of choices) {
    const label = e.title ?? e.name ?? e.entity_id
    labelCounts.set(label, (labelCounts.get(label) ?? 0) + 1)
  }
  const upload = async (image: File) => {
    setBusy(true)
    setError('')
    try {
      const { payload, facts } = await stageMediaFile(store, image)
      if (facts.kind !== 'image') throw new Error(t('aiworkspace.table.ref.invalidImage'))
      const id = randomId('a')
      const result = await store.submit({ editId: `asset:${id}`, label: image.name, operations: [
        { op: 'entity.create', type_id: 'buckyos.asset-ref', entity_id: id, parent_id: parent, order_key: orderKeyAfter(store, parent, null), payload },
      ] })
      if (result.status !== 'accepted' && result.status !== 'saved_locally') throw new Error(t('aiworkspace.blocks.asset.replaceRejected'))
      onChange({ entity_id: id })
    } catch (e) { setError(describeError(e)) }
    finally { setBusy(false) }
  }
  return <span className="aiws-ref-input" onKeyDown={(e) => { if (e.key === 'Escape' && onCancel) { e.stopPropagation(); onCancel() } }}>
    <ReferenceDisplay field={field} value={value} />
    <select aria-label={ariaLabel ?? t(field.ref_purpose ? referenceTypeLabels[field.ref_purpose] : 'aiworkspace.data.fieldType.objectRef')} autoFocus={autoFocus} disabled={disabled || busy}
      value={current} onChange={(e) => onChange(e.target.value ? { entity_id: e.target.value } : undefined)}>
      <option value="">{t('aiworkspace.data.value.notSet')}</option>
      {current && !choices.some((e) => e.entity_id === current) && <option disabled value={current}>{t('aiworkspace.table.ref.unavailable')}</option>}
      {choices.map((e) => {
        const label = e.title ?? e.name ?? e.entity_id
        return <option key={e.entity_id} value={e.entity_id}>{labelCounts.get(label)! > 1 ? `${label} · ${e.entity_id}` : label}</option>
      })}
    </select>
    {field.ref_purpose === 'image' && store.outline.get(parent)?.capabilities.includes('structure') && <>
      <button type="button" disabled={busy || disabled} aria-label={t('aiworkspace.table.ref.upload')} onClick={() => file.current?.click()}><Upload size={14} /></button>
      <input ref={file} hidden type="file" accept="image/*" onChange={(e) => { const image = e.target.files?.[0]; e.target.value = ''; if (image) void upload(image) }} />
    </>}
    {error && <span role="alert" className="aiws-error">{error}</span>}
  </span>
}
