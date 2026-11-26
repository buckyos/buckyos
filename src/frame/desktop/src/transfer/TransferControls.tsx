/* The Shell's transfer controls in a window's title bar (剪贴板规范 §5, §7): the "Drag to another app" handle of the
 * focused window when its App offers a selection, and the window menu with "Paste desktop object…" for windows that
 * can receive. Both work with a pointer, a finger (tap) and the keyboard. */
import { useMemo, useState, useSyncExternalStore, type PointerEvent as ReactPointerEvent } from 'react'
import { Menu, MenuItem } from '@mui/material'
import { GripVertical, MoreHorizontal } from 'lucide-react'
import { useI18n } from '../i18n/provider'
import { useSourceAppTitle } from './names'
import { desktopTransfers } from './records'
import { liveOfferOf } from './shell'
import { hasPendingOffer, startChoosing, startHandleGesture, startPaste } from './gesture'

const buttonClass = 'flex h-6 w-6 items-center justify-center rounded-[6px] border-0 bg-transparent text-[color:var(--cp-muted)] transition-colors duration-150 hover:bg-[color:color-mix(in_srgb,var(--cp-text)_8%,transparent)] hover:text-[color:var(--cp-text)] focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-[color:var(--cp-accent)]'

/** Shown by the title bar when `useWindowTransferControls` says so. */
export function TransferHandle({ windowId }: { windowId: string }) {
  const { t } = useI18n()
  const version = useSyncExternalStore(desktopTransfers.subscribe, desktopTransfers.getVersion)
  // eslint-disable-next-line react-hooks/exhaustive-deps -- the records changed
  const offer = useMemo(() => liveOfferOf(windowId), [windowId, version])
  const app = useSourceAppTitle(offer?.source.app)
  if (!offer) return null
  const label = t('transfer.handle.label', undefined, { count: offer.items?.length ?? offer.itemKeys.length, app })
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      data-testid="transfer-handle"
      className={`${buttonClass} cursor-grab touch-none active:cursor-grabbing`}
      onPointerDown={(event: ReactPointerEvent<HTMLButtonElement>) => {
        event.stopPropagation()
        startHandleGesture(windowId, event.nativeEvent, event.currentTarget)
      }}
      // A pointer's tap is handled on release; Enter or Space starts choosing a destination.
      onClick={(event) => { event.stopPropagation(); if (event.detail === 0) startChoosing(windowId, event.currentTarget) }}
    >
      <GripVertical className="size-[13px] stroke-[2]" />
    </button>
  )
}

/** Shown by the title bar when `useWindowTransferControls` says so. */
export function WindowTransferMenu({ windowId }: { windowId: string }) {
  const { t } = useI18n()
  const version = useSyncExternalStore(desktopTransfers.subscribe, desktopTransfers.getVersion)
  const [anchor, setAnchor] = useState<HTMLButtonElement | null>(null)
  // eslint-disable-next-line react-hooks/exhaustive-deps -- the records changed
  const pending = useMemo(() => hasPendingOffer(windowId), [windowId, version])
  return (
    <>
      <button
        type="button"
        aria-label={t('transfer.windowMenu')}
        title={t('transfer.windowMenu')}
        aria-haspopup="menu"
        aria-expanded={!!anchor}
        data-testid="window-menu"
        className={buttonClass}
        onPointerDown={(event) => event.stopPropagation()}
        onClick={(event) => { event.stopPropagation(); setAnchor(event.currentTarget) }}
      >
        <MoreHorizontal className="size-[13px] stroke-[2]" />
      </button>
      <Menu anchorEl={anchor} open={!!anchor} onClose={() => setAnchor(null)}>
        <MenuItem disabled={!pending} onClick={() => { const from = anchor; setAnchor(null); startPaste(windowId, from) }}>
          {t('transfer.pasteDesktop')}
        </MenuItem>
      </Menu>
    </>
  )
}
