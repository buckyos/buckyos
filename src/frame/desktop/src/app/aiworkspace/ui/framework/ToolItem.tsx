/* A tool item (UI 框架 PRD §3.3): an action in a tool zone — a single-icon button whose name is in its tooltip; with
 * several candidates a drop-down of them (one candidate runs directly). The same name, icon and disabled reason as
 * the action's main menu entry. An action that explains its reason (`explain`, a structural limit) looks disabled
 * and still takes the click: it opens a note with the reason, since a tooltip is no help on a touch screen. */

import { createElement, type ReactNode } from 'react'
import { Plus } from 'lucide-react'
import { MenuList, PopoverPanel, usePopover } from '../shell/popover'
import { candidateItem, type ToolItemModel } from './actions'
import { iconOf } from './icons'

export function ToolItem({ item, testId, size = 18, align = 'end', children }: { item: ToolItemModel; testId?: string; size?: number; align?: 'start' | 'end'; children?: ReactNode }) {
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const icon = typeof item.icon === 'string' ? item.icon : ''
  const explains = item.reason !== null && Boolean(item.def.explain)
  const menu = explains ? null : item.menu
  const disabled = item.reason !== null && !explains && !open
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-tool" aria-label={item.label} title={item.reason ?? item.tip} aria-pressed={item.pressed}
        aria-haspopup={explains ? 'dialog' : menu ? 'menu' : undefined} aria-expanded={explains || menu ? open : undefined} disabled={disabled} aria-disabled={explains || undefined}
        data-testid={testId} data-action={item.def.id} data-icon={icon} data-reason={item.reason ?? undefined}
        onClick={() => { if (explains || menu) toggle(); else item.run?.() }}>
        {children ?? createElement(iconOf(item.icon) ?? Plus, { size, 'aria-hidden': true })}
      </button>
      {open && explains && (
        <PopoverPanel align={align} label={item.label} testId={testId ? `${testId}-reason` : undefined} className="aiws-tool-reason">
          <p role="status">{item.reason}</p>
        </PopoverPanel>
      )}
      {open && menu && <MenuList items={menu.map(candidateItem)} label={item.label} className={`aiws-popover aiws-popover-${align}`} testId={testId ? `${testId}-list` : undefined} onDone={close} />}
    </span>
  )
}
