/* What a composite draws around the Blocks of its slots (扩展控件 P1, R23: chrome is template, the content is real
 * Blocks): its background, corner radius and border, the title band (`{{config.<key>}}` filled from the instance's
 * config over the definition's defaults; hidden once the title's slot holds something), and the `empty` text while
 * it holds nothing. A definition the outline does not carry (not readable, gone) shows a plain dashed box. */

import { useStore } from '../../../state/hooks'
import { compositeOf, dimension, showsTitle, templated, TITLE_BAND, DEFAULT_PADDING, paddingOf } from '@controls/composite'
import { useAiwsText } from '../../../useAiwsText'
import type { ControlChromeProps } from '../registry'

export function CompositeChrome({ container }: ControlChromeProps) {
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
  const empty = children.length === 0 ? (text(chrome.empty) || t('aiworkspace.ext.composite.empty')) : ''
  return (
    <div className={`aiws-composite-chrome${composition ? '' : ' is-unknown'}`} data-testid={`aiws-composite-chrome-${id}`} data-def={container.entity.def_id ?? ''}
      style={{ width: Math.max(1, box.w), height: Math.max(1, box.h), borderRadius: radius, ...(background ? { background } : {}), ...(border ? { borderColor: border } : {}) }}>
      {title && <div className="aiws-composite-title" style={{ left: pad.l, top: pad.t, height: TITLE_BAND - 4, right: pad.r }} data-testid={`aiws-composite-title-${id}`}>{title}</div>}
      {empty && <div className="aiws-composite-empty" style={{ inset: `${pad.t + (title ? TITLE_BAND : 0)}px ${pad.r || DEFAULT_PADDING}px ${pad.b}px ${pad.l}px` }}>{empty}</div>}
      {!composition && <div className="aiws-composite-empty">{t('aiworkspace.ext.composite.definitionMissing')}</div>}
    </div>
  )
}
