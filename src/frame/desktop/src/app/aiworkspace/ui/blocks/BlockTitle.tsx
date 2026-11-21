import { useState } from 'react'
import type { EntityEnvelope } from '../../api/types'
import { aiwsT } from '../../i18n'
import { useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'

/** The heading line of a table, record or wish on a page: the Block's title, renamed in place (an empty one clears it). */
export function BlockTitle({ cell, title, canRename, select, finishRename }: {
  cell: EntityEnvelope; title: string; canRename: boolean; select?: (clickCount: number) => void; finishRename?: () => void
}) {
  const store = useStore()
  const { t } = useAiwsText()
  const [text, setText] = useState<string | null>(null)
  const save = async (next: string) => {
    setText(null)
    if (next === (cell.title ?? title)) return
    const read = await store.session.read<{ key_revs: Record<string, number> }>(cell.entity_id)
    const rev = read.content.key_revs.title ?? 0
    void store.submit({
      editId: `key:${cell.entity_id}:title`, label: next ? aiwsT('aiworkspace.canvas.blockTitleLabel', { title: next }) : aiwsT('aiworkspace.canvas.blockTitleClearedLabel'), mine: next, hasMine: true,
      operations: [next === ''
        ? { op: 'entity.unset_keys', entity_id: cell.entity_id, keys: [{ key: 'title', expect: { rev } }] }
        : { op: 'entity.set_keys', entity_id: cell.entity_id, keys: [{ key: 'title', value: next, expect: { rev } }] }],
    })
  }
  if (text !== null) {
    return (
      <input className="aiws-fblock-title aiws-fblock-title-input" aria-label={t('aiworkspace.canvas.blockTitle')} data-row-title="" data-testid={`aiws-cell-title-input-${cell.entity_id}`} autoFocus value={text} placeholder={t('aiworkspace.canvas.noTitle')}
        onChange={(event) => setText(event.target.value)} onBlur={() => { void save(text.trim()) }}
        onKeyDown={(event) => {
          if (event.key !== 'Enter' && event.key !== 'Escape') return
          event.preventDefault()
          event.stopPropagation()
          if (event.key === 'Enter') void save(text.trim())
          else setText(null)
          finishRename?.()
        }} />
    )
  }
  return (
    <div className={`aiws-fblock-title${canRename ? ' is-editable' : ''}${title ? '' : ' is-empty'}`} data-row-title="" data-testid={`aiws-cell-title-${cell.entity_id}`}
      title={canRename && !select ? t('aiworkspace.canvas.clickToRename') : undefined} tabIndex={canRename || select ? 0 : undefined} role={canRename || select ? 'button' : undefined}
      onClick={(event) => { if (select) select(event.detail); else if (canRename) setText(title) }}
      onDoubleClick={(e) => { e.stopPropagation(); if (select && canRename) setText(title) }}
      onKeyDown={(e) => { if (e.key === 'Enter' && canRename) { e.preventDefault(); e.stopPropagation(); setText(title) } }}>
      {title || t('aiworkspace.canvas.noTitle')}
    </div>
  )
}
