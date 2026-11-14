/* How big an object is, for its properties and the status bar (UI 框架 PRD §5.4, §5.5): a table's rows (the query's
 * total), a directory's members, an asset's bytes; nothing for the rest. */

import { useCallback } from 'react'
import type { AssetContent, EntityEnvelope } from '../../api/types'
import { useLoad, useStore, useVersion } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'

export interface ObjectSize { kind: 'rows' | 'members' | 'bytes'; count: number; text: string }

export function useObjectSize(entity: EntityEnvelope | undefined): ObjectSize | null {
  const store = useStore()
  const { t } = useAiwsText()
  const id = entity?.entity_id ?? ''
  const type = entity?.type_id ?? ''
  const version = useVersion(`e:${id}`)
  const load = useCallback(async (): Promise<{ kind: ObjectSize['kind']; count: number } | null> => {
    if (!id) return null
    if (type === 'buckyos.table-source') return { kind: 'rows', count: (await store.session.query({ source_id: id, limit: 1 })).total }
    if (type === 'buckyos.dir') {
      const read = await store.readBatched<{ payload: { members?: Record<string, unknown> } }>(id)
      return { kind: 'members', count: Object.keys(read.content.payload.members ?? {}).length }
    }
    if (type === 'buckyos.asset-ref') {
      const read = await store.readBatched<AssetContent>(id)
      return typeof read.content.payload.size === 'number' ? { kind: 'bytes', count: read.content.payload.size } : null
    }
    return null
  }, [store, id, type])
  const read = useLoad(load, `${id}:${version}:${entity?.content_rev ?? 0}`)
  const data = read.data
  if (!data || !entity) return null
  const text = data.kind === 'rows' ? t('aiworkspace.datamode.rows', { count: data.count })
    : data.kind === 'members' ? t('aiworkspace.datamode.members', { count: data.count })
      : data.count >= 1024 * 1024 ? `${(data.count / 1024 / 1024).toFixed(1)} MiB` : data.count >= 1024 ? `${(data.count / 1024).toFixed(1)} KiB` : t('aiworkspace.blocks.asset.bytes', { count: data.count })
  return { ...data, text }
}
