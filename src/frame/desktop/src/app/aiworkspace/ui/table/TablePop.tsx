/* eslint-disable react-refresh/only-export-components -- the editor's pop-overs and their helpers */
/* The table editor's pop-overs (UI 框架 PRD §6.1, §7.3): a menu, a confirmation or a panel beside the button that opened
 * it; on a phone the same content as a sheet from the bottom. Built on the data controls' pop-over. */

import { useState, type ReactNode } from 'react'
import { ControlMenu, ControlPopover, anchorOf, type AnchorRect, type MenuItem } from '../controls/data/ControlPopover'
import { usePhone } from '../shell/shellContext'
import { useAiwsText } from '../../useAiwsText'

export type { AnchorRect, MenuItem }
export { anchorOf }

export function TablePop({ anchor, onClose, label, children, testId, width = 320 }: { anchor: AnchorRect; onClose: () => void; label: string; children: ReactNode; testId?: string; width?: number }) {
  const phone = usePhone()
  return (
    <ControlPopover anchor={anchor} onClose={onClose} label={label} sheet={phone} width={width} side="below" testId={testId}>
      <div className="aiws-table-pop">{children}</div>
    </ControlPopover>
  )
}

/** A menu in a pop-over (a sheet on a phone); the items close it when chosen. */
export function TableMenu({ anchor, onClose, label, items, testId }: { anchor: AnchorRect; onClose: () => void; label: string; items: (MenuItem | 'separator')[]; testId?: string }) {
  return (
    <TablePop anchor={anchor} onClose={onClose} label={label} testId={testId} width={260}>
      <div className="aiws-table-pop-title">{label}</div>
      <ControlMenu items={items} onClose={onClose} label={label} />
    </TablePop>
  )
}

/** Ask before something that removes data; undo still takes it back. */
export function ConfirmPop({ anchor, title, detail, confirm, onConfirm, onClose }: { anchor: AnchorRect; title: string; detail?: string; confirm: string; onConfirm: () => void; onClose: () => void }) {
  const { t } = useAiwsText()
  return (
    <TablePop anchor={anchor} onClose={onClose} label={title} testId="aiws-table-confirm" width={300}>
      <div className="aiws-table-pop-title">{title}</div>
      {detail && <div className="aiws-muted">{detail}</div>}
      <div className="aiws-table-pop-actions">
        <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
        <button type="button" className="is-danger" data-testid="aiws-table-confirm-ok" autoFocus onClick={() => { onClose(); onConfirm() }}>{confirm}</button>
      </div>
    </TablePop>
  )
}

/** Which pop-over is open, and where (the element that opened it). */
export function usePop<K extends string>() {
  const [pop, setPop] = useState<{ kind: K; anchor: AnchorRect } | null>(null)
  return {
    pop,
    open: (kind: K, target: Element | null) => setPop({ kind, anchor: anchorOf(target as HTMLElement | null) }),
    openAt: (kind: K, anchor: AnchorRect) => setPop({ kind, anchor }),
    close: () => setPop(null),
  }
}
