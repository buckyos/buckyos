/* The Shell's transfer layer (剪贴板规范 §5, §7): the drag preview with the target App, the action it named and the
 * item count; choosing a destination by tapping or from a list; the "Paste desktop object…" dialog; the result,
 * readable by screen readers. Drawn over the windows only while a transfer is in progress. */
import { useEffect, useRef, useSyncExternalStore } from 'react'
import { createPortal } from 'react-dom'
import { Button, Dialog, DialogActions, DialogContent, DialogTitle } from '@mui/material'
import { X } from 'lucide-react'
import { useI18n } from '../i18n/provider'
import { useDesktopUISnapshot } from '../models/DesktopUIDataModel'
import { cancelGesture, chooseAt, chooseDestination, confirmPaste, dismissNotice, hoverChoice, retryNotice, transferUi, type TransferHit } from './gesture'
import { useSourceAppTitle, useWindowNames } from './names'
import { subscribeWindowTransferTargets, windowTransferTarget, windowTransferTargetsVersion } from './targets'

const LAYER_Z = 1400

function Outline({ hit, accepted }: { hit: TransferHit; accepted: boolean }) {
  return (
    <div
      aria-hidden="true"
      data-testid="transfer-target-outline"
      data-accepted={accepted}
      className="pointer-events-none fixed rounded-[12px]"
      style={{
        left: hit.rect.left, top: hit.rect.top, width: hit.rect.width, height: hit.rect.height,
        boxShadow: accepted ? 'inset 0 0 0 2px var(--cp-accent)' : 'inset 0 0 0 2px color-mix(in srgb, var(--cp-text) 30%, transparent)',
        background: accepted ? 'color-mix(in srgb, var(--cp-accent) 6%, transparent)' : 'transparent',
      }}
    />
  )
}

const cardClass = 'rounded-[10px] border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] text-[color:var(--cp-text)] shadow-[var(--cp-panel-shadow)]'

export function ShellTransferLayer() {
  const { t } = useI18n()
  const { gesture, notice } = useSyncExternalStore(transferUi.subscribe, transferUi.getSnapshot)
  const names = useWindowNames()
  const sourceApp = useSourceAppTitle(gesture?.sourceApp)
  const { runtime } = useDesktopUISnapshot()
  useSyncExternalStore(subscribeWindowTransferTargets, windowTransferTargetsVersion)
  const bar = useRef<HTMLDivElement>(null)

  const choosing = gesture?.mode === 'choose'
  useEffect(() => {
    if (!choosing) return
    bar.current?.querySelector<HTMLButtonElement>('button')?.focus()
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); cancelGesture() } }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [choosing])

  // A result stays until it is dismissed when it offers a check; otherwise it goes after a while.
  useEffect(() => {
    if (!notice || notice.retry) return
    const timer = setTimeout(dismissNotice, 6000)
    return () => clearTimeout(timer)
  }, [notice])

  const count = gesture?.count ?? 0
  // On a phone the other windows are not loaded, so whether they receive is only known after switching to one.
  const destinations = gesture?.mode === 'choose'
    ? [...runtime.windows].filter((w) => w.id !== gesture.op.sourceWindowId && (gesture.phone || (w.state !== 'minimized' && windowTransferTarget(w.id)))).sort((a, b) => b.lastActive - a.lastActive)
    : []

  const content = (
    <div data-transfer-layer="" className="contents">
      {gesture?.mode === 'drag' ? (
        <div className="pointer-events-none fixed inset-0" style={{ zIndex: LAYER_Z, cursor: gesture.answer ? 'copy' : 'no-drop' }}>
          {gesture.hit ? <Outline hit={gesture.hit} accepted={!!gesture.answer} /> : null}
          <div
            role="status"
            aria-live="polite"
            data-testid="transfer-preview"
            data-state={gesture.answer ? 'accepted' : gesture.answer === null ? 'refused' : 'idle'}
            className={`${cardClass} fixed max-w-[280px] px-3 py-2 text-xs`}
            style={{ left: gesture.x + 14, top: gesture.y + 14, opacity: gesture.answer === null || !gesture.hit ? 0.75 : 1 }}
          >
            <p className="font-medium">
              {!gesture.hit
                ? t('transfer.preview.other')
                : gesture.answer
                  ? `${names(gesture.hit.windowId).app}: ${gesture.answer.action || t('transfer.pasteDesktop.confirm')}`
                  : gesture.answer === null
                    ? t('transfer.notAccepted', undefined, { app: names(gesture.hit.windowId).app })
                    : t('transfer.checking')}
            </p>
            <p className="mt-0.5 text-[color:var(--cp-muted)]">{t('transfer.preview.count', undefined, { count, app: sourceApp })}</p>
          </div>
        </div>
      ) : null}

      {gesture?.mode === 'switching' ? (
        <div role="status" aria-live="polite" data-testid="transfer-switching" className={`${cardClass} fixed bottom-5 left-1/2 -translate-x-1/2 px-4 py-2 text-sm`} style={{ zIndex: LAYER_Z + 1 }}>
          {t('transfer.checking')}
        </div>
      ) : null}

      {gesture?.mode === 'choose' ? (
        <>
          {gesture.phone ? null : <div
            data-testid="transfer-choose-overlay"
            className="fixed inset-0 cursor-crosshair"
            style={{ zIndex: LAYER_Z }}
            onPointerMove={(event) => { if (event.pointerType === 'mouse') hoverChoice(event.clientX, event.clientY) }}
            onClick={(event) => chooseAt(event.clientX, event.clientY)}
          />}
          {gesture.hover ? <Outline hit={gesture.hover} accepted /> : null}
          <div
            ref={bar}
            role="dialog"
            aria-modal="false"
            aria-label={t('transfer.choose.title', undefined, { count, app: sourceApp })}
            data-testid="transfer-choose"
            className={`${cardClass} fixed left-1/2 w-[min(480px,calc(100vw-32px))] -translate-x-1/2 p-3 text-sm ${gesture.phone ? 'bottom-4' : 'top-3'}`}
            style={{ zIndex: LAYER_Z + 1 }}
          >
            <p className="font-medium">{t('transfer.choose.title', undefined, { count, app: sourceApp })}</p>
            <p className="mt-0.5 text-xs text-[color:var(--cp-muted)]">{!destinations.length ? t('transfer.choose.none') : gesture.phone ? t('transfer.choose.list') : t('transfer.choose.hint')}</p>
            {gesture.refused ? (
              <p role="alert" className="mt-1 text-xs text-[color:var(--cp-danger,#c0392b)]">{t('transfer.notAccepted', undefined, { app: names(gesture.refused).app })}</p>
            ) : null}
            <div className="mt-2 flex flex-wrap gap-2">
              {destinations.map((w) => {
                const name = names(w.id)
                return (
                  <Button key={w.id} size="small" variant="outlined" onClick={() => void chooseDestination(w.id, 'focus')}>
                    {t('transfer.choose.window', undefined, { window: name.window, app: name.app })}
                  </Button>
                )
              })}
              <Button size="small" onClick={() => cancelGesture()}>{t('transfer.cancel')}</Button>
            </div>
          </div>
        </>
      ) : null}

      <Dialog
        open={gesture?.mode === 'paste'}
        onClose={() => cancelGesture({ silent: true })}
        aria-labelledby="transfer-paste-title"
      >
        {gesture?.mode === 'paste' ? (
          <>
            <DialogTitle id="transfer-paste-title">{t('transfer.pasteDesktop.title')}</DialogTitle>
            <DialogContent>
              <p className="text-sm">{t('transfer.pasteDesktop.from', undefined, { app: sourceApp, count })}</p>
              <p className="mt-1 text-sm">{t('transfer.pasteDesktop.to', undefined, { window: names(gesture.windowId).window })}</p>
              <p className="mt-3 text-sm text-[color:var(--cp-muted)]" role="status" aria-live="polite">
                {gesture.probing ? t('transfer.checking') : gesture.answer ? gesture.answer.action : t('transfer.notAccepted', undefined, { app: names(gesture.windowId).app })}
              </p>
            </DialogContent>
            <DialogActions>
              <Button onClick={() => cancelGesture({ silent: true })}>{t('transfer.cancel')}</Button>
              <Button variant="contained" disabled={!gesture.answer} onClick={confirmPaste}>{t('transfer.pasteDesktop.confirm')}</Button>
            </DialogActions>
          </>
        ) : null}
      </Dialog>

      {notice ? (
        <div
          key={notice.id}
          role={notice.alert ? 'alert' : 'status'}
          aria-live={notice.alert ? 'assertive' : 'polite'}
          data-testid="transfer-notice"
          className={`${cardClass} fixed bottom-5 left-1/2 flex w-max max-w-[calc(100vw-32px)] -translate-x-1/2 items-center gap-3 px-4 py-2 text-sm`}
          style={{ zIndex: LAYER_Z + 2 }}
        >
          <span>{t(notice.key, undefined, { ...notice.vars, app: names(notice.windowId).app })}</span>
          {notice.retry ? <Button size="small" onClick={() => void retryNotice(notice.retry!)}>{t('transfer.retry')}</Button> : null}
          <button type="button" aria-label={t('transfer.dismiss')} className="flex size-6 items-center justify-center rounded-[6px] text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]" onClick={dismissNotice}>
            <X className="size-[13px]" />
          </button>
        </div>
      ) : null}
    </div>
  )
  return createPortal(content, document.body)
}
