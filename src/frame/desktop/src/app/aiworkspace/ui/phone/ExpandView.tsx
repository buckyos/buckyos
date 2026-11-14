/* "展开" on a phone (UI 框架 PRD §6.4, §7.1): the data a Block of a free canvas shows, full screen, in its standard
 * editor — the same one as its data mode tab: the table editor (records as cards, the record form), the directory
 * editor, the rich text editor (its format tools over the keyboard), the record form. The top bar: back to the canvas,
 * the object's name, ⋯ (the Block's menu). The editor gets the whole height that is left. Writing follows the data's
 * rights and the window's read-only state (Q12): the canvas stays in view mode, and shows the changes when it is back. */

import { useCallback, useEffect } from 'react'
import { ArrowLeft, Ellipsis } from 'lucide-react'
import type { Reference } from '../../api/types'
import { useDirectReadOnly, useOutlineVersion, useReadOnlyReason, useStore, useWorkspaceUi } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { RecordCell, RichTextCell } from '../blocks/editors'
import { readOnlyReason } from '../datamode/model'
import { DirectoryEditor } from '../directory/DirectoryEditor'
import { useOverlayMounted } from '../shell/popover'
import { StandardTableEditor } from '../table/StandardTableEditor'
import { FormatBar, FormatScope } from './FormatBar'
import './phone.css'

export interface ExpandTarget { cellId: string; sourceId: string }

export function ExpandView({ target, onClose, onMore }: { target: ExpandTarget; onClose: () => void; onMore: (anchor: HTMLElement) => void }) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const { t } = useAiwsText()
  useOutlineVersion()
  useOverlayMounted(true)
  const windowReason = useReadOnlyReason()
  const readOnly = useDirectReadOnly()
  const source = store.outline.get(target.sourceId)
  const cell = store.outline.get(target.cellId)
  const name = source?.title ?? source?.name ?? cell?.title ?? t('aiworkspace.phone.untitled')
  const reason = readOnlyReason(store, source, windowReason)
  const renderEmbed = useCallback((reference: Reference) => ui.renderCell(reference.entity_id, 1), [ui])
  // the data went away (deleted elsewhere): back to the canvas
  const gone = !source || source.deleted
  useEffect(() => { if (gone) onClose() }, [gone, onClose])
  if (gone) return null
  let editor
  switch (source.type_id) {
    case 'buckyos.table-source': editor = <StandardTableEditor sourceId={source.entity_id} source={source} size="fullscreen" readOnly={readOnly || Boolean(source.source)} readOnlyReason={reason} />; break
    case 'buckyos.dir': editor = <DirectoryEditor key={source.entity_id} entityId={source.entity_id} size="full" readOnlyReason={readOnly ? reason : null} cellId={target.cellId} />; break
    case 'buckyos.richtext': editor = <FormatScope id={`expand:${source.entity_id}`}><div className="aiws-phone-expand-text"><RichTextCell entity={source} entityId={source.entity_id} renderEmbed={renderEmbed} /></div></FormatScope>; break
    case 'buckyos.record': editor = <div className="aiws-phone-expand-text"><RecordCell entityId={source.entity_id} entity={source} readOnly={readOnly} /></div>; break
    default: editor = <div className="aiws-muted aiws-phone-expand-none">{t('aiworkspace.phone.noEditor')}</div>
  }
  return (
    <div className="aiws-phone-expand" role="dialog" aria-modal="true" aria-label={t('aiworkspace.phone.expanded', { name })} data-testid="aiws-phone-expand" data-source-id={source.entity_id} data-type={source.type_id}
      onPointerDown={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key === 'Escape' && !event.defaultPrevented) { event.stopPropagation(); onClose() } }}>
      <div className="aiws-phone-expand-bar">
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.phone.backToCanvas')} title={t('aiworkspace.phone.backToCanvas')} data-testid="aiws-phone-expand-back" onClick={onClose}><ArrowLeft size={20} /></button>
        <b className="aiws-phone-expand-name" data-testid="aiws-phone-expand-name">{name}</b>
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.canvas.near.more')} title={t('aiworkspace.canvas.near.more')} aria-haspopup="menu" data-testid="aiws-phone-expand-more" onClick={(event) => onMore(event.currentTarget)}><Ellipsis size={20} /></button>
      </div>
      <div className="aiws-phone-expand-body">{editor}</div>
      <FormatBar />
    </div>
  )
}
