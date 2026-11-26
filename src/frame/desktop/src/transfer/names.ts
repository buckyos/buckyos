/* Names the Shell's transfer UI shows: the App a selection comes from, the window it goes to. */
import { useCallback, useMemo, useSyncExternalStore } from 'react'
import { useDesktopUISnapshot } from '../models/DesktopUIDataModel'
import { useAppText } from '../presentation/appText'
import { desktopTransfers } from './records'
import { liveOfferOf } from './shell'
import { subscribeWindowTransferTargets, windowTransferTarget, windowTransferTargetsVersion } from './targets'
import { hasPendingOffer, startChoosing, startPaste } from './gesture'

/** The App named by a record's source (`appInstanceId` or desktop id). */
export function useSourceAppTitle(source: string | undefined): string {
  const { apps } = useDesktopUISnapshot()
  const appText = useAppText()
  const app = source ? apps.find((a) => (a.appInstanceId ?? a.id) === source || a.id === source) : undefined
  return app ? appText(app).title : source?.split('@')[0] ?? ''
}

/** A window's App title and its own title (the open document), by window id. */
export function useWindowNames(): (windowId: string | undefined) => { app: string; window: string } {
  const { apps, runtime } = useDesktopUISnapshot()
  const appText = useAppText()
  return useCallback((windowId) => {
    const record = runtime.windows.find((w) => w.id === windowId)
    const app = record ? apps.find((a) => a.id === record.appId) : undefined
    const title = app ? appText(app).title : ''
    return { app: title, window: record?.title || title }
  }, [apps, runtime.windows, appText])
}

/** Which transfer controls a window's title bar shows: the handle (focused window with a live selection) and the
 * window menu (a window that can receive). */
export function useWindowTransferControls(windowId: string, isFront: boolean): { handle: boolean; menu: boolean } {
  const records = useSyncExternalStore(desktopTransfers.subscribe, desktopTransfers.getVersion)
  const targets = useSyncExternalStore(subscribeWindowTransferTargets, windowTransferTargetsVersion)
  // eslint-disable-next-line react-hooks/exhaustive-deps -- recomputed when records or registrations change
  return useMemo(() => ({ handle: isFront && !!liveOfferOf(windowId), menu: !!windowTransferTarget(windowId) }), [windowId, isFront, records, targets])
}

/** The transfer commands of the window in front, for the phone's app menu (no title bar there): "Send to another
 * app…" when it offers a selection, "Paste desktop object…" when it receives and another window offers one. */
export function useFrontWindowTransferCommands(): Array<{ key: string; labelKey: string; run: (from: HTMLElement | null) => void }> {
  const { runtime } = useDesktopUISnapshot()
  const front = runtime.windows.filter((w) => w.state !== 'minimized').reduce<(typeof runtime.windows)[number] | undefined>((top, w) => (!top || w.zIndex > top.zIndex ? w : top), undefined)
  const controls = useWindowTransferControls(front?.id ?? '', true)
  const records = useSyncExternalStore(desktopTransfers.subscribe, desktopTransfers.getVersion)
  // eslint-disable-next-line react-hooks/exhaustive-deps -- recomputed when records change
  const pending = useMemo(() => !!front && hasPendingOffer(front.id), [front, records])
  if (!front) return []
  return [
    ...(controls.handle ? [{ key: 'send', labelKey: 'transfer.send', run: (from: HTMLElement | null) => startChoosing(front.id, from, true) }] : []),
    ...(controls.menu && pending ? [{ key: 'paste', labelKey: 'transfer.pasteDesktop', run: (from: HTMLElement | null) => startPaste(front.id, from) }] : []),
  ]
}
