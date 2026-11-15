/* eslint-disable react-refresh/only-export-components -- the dialog frame and the hooks that ask with it */
/* The frame of a wish card's windows (许愿格交互升级 §3.4): over the whole window wherever it is opened from (a Block on a
 * canvas, a flow page, the data mode), the whole screen on a phone. And the questions asked with it: before a change that
 * makes later stages out of date (§3.5), before a rollback that loses later changes. */

import { useCallback, useState, type ReactNode, type SyntheticEvent } from 'react'
import { createPortal } from 'react-dom'
import { X } from 'lucide-react'
import type { FreshnessInfo, WishPayload } from '../../api/types'
import type { AiwsText } from '../../i18n'
import { useAiwsText } from '../../useAiwsText'
import { useOverlayMounted } from '../shell/popover'
import { usePhone } from '../shell/shellContext'

const stop = (event: SyntheticEvent) => event.stopPropagation()

/** A dialog of the wish card: React events stay in it (the canvas behind takes no press, click or menu from it). */
export function WishDialog({ title, kind, onClose, children, actions, wide = false }: { title: string; kind: string; onClose: () => void; children: ReactNode; actions?: ReactNode; wide?: boolean }) {
  const { t } = useAiwsText()
  const phone = usePhone()
  useOverlayMounted(true)
  const host = (document.querySelector('.aiws-root') as HTMLElement | null) ?? document.body
  return createPortal(
    <div className={`aiws-modal-backdrop aiws-wd-backdrop${phone ? ' is-phone' : ''}`}
      onPointerDown={(event) => { event.stopPropagation(); if (event.target === event.currentTarget) onClose() }}
      onPointerUp={stop} onPointerMove={stop} onClick={stop} onDoubleClick={stop} onContextMenu={stop}
      onKeyDown={(event) => { event.stopPropagation(); if (event.key === 'Escape') onClose() }}>
      <div className={`aiws-dialog aiws-wd${wide ? ' is-wide' : ''}`} role="dialog" aria-modal="true" aria-label={title} data-testid="aiws-wish-dialog" data-kind={kind}>
        <div className="aiws-dialog-head">
          <b className="aiws-wd-title">{title}</b>
          <span className="aiws-grow" />
          <button type="button" className="aiws-tool" aria-label={t('aiworkspace.common.close')} title={t('aiworkspace.common.close')} data-testid="aiws-wish-dialog-close" onClick={onClose}><X size={18} /></button>
        </div>
        <div className="aiws-wd-body">{children}</div>
        {actions && <div className="aiws-dialog-actions aiws-wd-actions">{actions}</div>}
      </div>
    </div>,
    host,
  )
}

interface Ask { title: string; body: string; items?: string[]; ok: string; cancel: string; testId: string; resolve: (yes: boolean) => void }

/** A yes/no question in a wish dialog: `confirm` resolves true for the first button, false for the other or closing. */
export function useConfirm(): { node: ReactNode; confirm: (ask: Omit<Ask, 'resolve'>) => Promise<boolean> } {
  const [ask, setAsk] = useState<Ask | null>(null)
  const confirm = useCallback((next: Omit<Ask, 'resolve'>) => new Promise<boolean>((resolve) => setAsk({ ...next, resolve })), [])
  const answer = (yes: boolean) => { ask?.resolve(yes); setAsk(null) }
  const node = ask ? (
    <WishDialog title={ask.title} kind={ask.testId} onClose={() => answer(false)} actions={<>
      <button type="button" data-testid={`${ask.testId}-cancel`} onClick={() => answer(false)}>{ask.cancel}</button>
      <button type="button" className="is-primary" data-testid={`${ask.testId}-ok`} autoFocus onClick={() => answer(true)}>{ask.ok}</button>
    </>}>
      <div data-testid={ask.testId}>
        <p>{ask.body}</p>
        {ask.items && ask.items.length > 0 && <ul className="aiws-wd-list">{ask.items.map((item, i) => <li key={i}>{item}</li>)}</ul>}
      </div>
    </WishDialog>
  ) : null
  return { node, confirm }
}

// ---- what a change makes out of date (§3.5)

export type InvalidationKind = 'request' | 'macro'

/** What changing the request side (the request, knowledge, refinements, inputs) or the macro by hand makes out of date,
 * as the question says it; null when nothing after it would be (no current requirements; no results). */
export function invalidationText(kind: InvalidationKind, payload: WishPayload | undefined, freshness: FreshnessInfo | undefined, text: AiwsText): string | null {
  if (!payload) return null
  const results = (payload.history?.results_count ?? 0) > 0 || Boolean(payload.last_run)
  if (kind === 'macro') return results ? text.t('aiworkspace.wishcard.invalidate.results') : null
  const current = Boolean(payload.analysis) && !freshness?.needs_analysis
  if (!current) return null
  const head = text.t(payload.program ? 'aiworkspace.wishcard.invalidate.analysisAndMacro' : 'aiworkspace.wishcard.invalidate.analysis')
  return results ? `${head}${text.t('aiworkspace.wishcard.invalidate.resultsTail')}` : head
}

/** Ask before a change of `kind`: resolves true at once when nothing would be made out of date. */
export function useInvalidation(payload: WishPayload | undefined, freshness: FreshnessInfo | undefined): { node: ReactNode; ask: (kind: InvalidationKind) => Promise<boolean> } {
  const text = useAiwsText()
  const { node, confirm } = useConfirm()
  const ask = useCallback((kind: InvalidationKind) => {
    const body = invalidationText(kind, payload, freshness, text)
    if (!body) return Promise.resolve(true)
    return confirm({ title: text.t('aiworkspace.wishcard.invalidate.title'), body, ok: text.t('aiworkspace.wishcard.invalidate.continue'), cancel: text.t('aiworkspace.common.cancel'), testId: 'aiws-wish-invalidate' })
  }, [payload, freshness, text, confirm])
  return { node, ask }
}
