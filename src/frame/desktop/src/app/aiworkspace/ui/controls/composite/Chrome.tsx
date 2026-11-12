/* What a composite draws around the Blocks of its slots (扩展控件 P1, R23: chrome is template, the content is real
 * Blocks): its background, corner radius and border, the title band (`{{config.<key>}}` filled from the instance's
 * config over the definition's defaults; hidden once the title's slot holds something), and the `empty` text while
 * it holds nothing. A definition the outline does not carry (not readable, gone) shows a plain dashed box. */

import { Upload } from 'lucide-react'
import { useEntity, useStore } from '../../../state/hooks'
import type { CompositionMedia } from '@controls/composite'
import { placeBroadcast } from '../../media/BroadcastMedia'
import { useFilePicker } from '../../shell/filePicker'
import { compositeOf, dimension, showsTitle, templated, TITLE_BAND, DEFAULT_PADDING, paddingOf, unknownAlgorithm } from '@controls/composite'
import { useAiwsText } from '../../../useAiwsText'
import type { ControlChromeProps } from '../registry'
import { CompositeProjections } from './Projections'

export function CompositeChrome({ container, mode }: ControlChromeProps) {
  const { t } = useAiwsText()
  const store = useStore()
  const id = container.entity.entity_id
  const box = container.rect
  const { composition, config } = compositeOf(store.outline, id)
  const chrome = composition?.chrome ?? {}
  const children = store.outline.childrenOf(id).filter((c) => !c.deleted)
  const text = (v: unknown) => { const s = templated(v, config); return typeof s === 'string' || typeof s === 'number' ? String(s) : '' }
  const background = text(chrome.background) || undefined
  const border = text(chrome.border) || undefined
  const radius = dimension(chrome.radius, config, 10)
  const pad = paddingOf(composition, config)
  const title = showsTitle(composition, children) ? text(chrome.title?.text) : ''
  const empty = children.length === 0 && !Object.keys(composition?.projections ?? {}).length ? (text(chrome.empty) || t('aiworkspace.ext.composite.empty')) : ''
  // a layout this host does not ship (Q33): laid out as a group, and said so
  const unknown = unknownAlgorithm(composition)
  return (
    <div className={`aiws-composite-chrome${composition && !unknown ? '' : ' is-unknown'}`} data-testid={`aiws-composite-chrome-${id}`} data-def={container.entity.def_id ?? ''}
      style={{ width: Math.max(1, box.w), height: Math.max(1, box.h), borderRadius: radius, ...(background ? { background } : {}), ...(border ? { borderColor: border } : {}) }}>
      {title && <div className="aiws-composite-title" style={{ left: pad.l, top: pad.t, height: TITLE_BAND - 4, right: pad.r }} data-testid={`aiws-composite-title-${id}`}>{title}</div>}
      {empty && <div className="aiws-composite-empty" style={{ inset: `${pad.t + (title ? TITLE_BAND : 0)}px ${pad.r || DEFAULT_PADDING}px ${pad.b}px ${pad.l}px` }}>{empty}</div>}
      {!composition && <div className="aiws-composite-empty">{t('aiworkspace.ext.composite.definitionMissing')}</div>}
      {unknown && <div className="aiws-composite-unknown" data-testid={`aiws-composite-unknown-${id}`}>{t('aiworkspace.ext.composite.unknownLayout', { name: unknown })}</div>}
      {composition?.media && !children.some((c) => c.slot === composition.media!.slot) && <EmptyMedia containerId={id} media={composition.media} />}
      {composition?.projections && <CompositeProjections containerId={id} regions={container.control?.placed.solved.regions} mode={mode} />}
      {(mode === 'edit' || mode === 'presentation_edit') && Object.values(composition?.slots ?? {}).some((s) => s.visible) && (
        <span className="aiws-composite-limited" data-testid={`aiws-composite-limited-${id}`}>{t('aiworkspace.ext.composite.timeLimited')}</span>
      )}
    </div>
  )
}

/** An empty media slot (G17, an empty TV screen): who may change the composite chooses the video or sound — from the
 * Zone first (the system picker; this device is a choice in it). */
function EmptyMedia({ containerId, media }: { containerId: string; media: CompositionMedia }) {
  const { t } = useAiwsText()
  const store = useStore()
  const container = useEntity(containerId)
  const picker = useFilePicker((message) => store.notify('error', message))
  if (!container?.capabilities.includes('update') || !container.capabilities.includes('structure')) return null
  const choose = async () => {
    const file = await picker.file('video/*,audio/*')
    if (file) await placeBroadcast(store, containerId, media, file, t('aiworkspace.broadcast.replace')).catch((error: unknown) => store.notify('error', error instanceof Error ? error.message : String(error)))
  }
  return (
    <button type="button" className="aiws-broadcast-choose" data-role="editor" data-testid={`aiws-broadcast-choose-${containerId}`} onPointerDown={(e) => e.stopPropagation()} onClick={() => void choose()}>
      <Upload size={14} aria-hidden="true" />{t('aiworkspace.broadcast.choose')}
    </button>
  )
}
