/* A tool item (UI 框架 PRD §3.3): an action in a tool zone — a single-icon button whose name is in its tooltip; with
 * several candidates a drop-down of them (one candidate runs directly). The same name, icon and disabled reason as
 * the action's main menu entry. */

import { createElement } from 'react'
import { Plus } from 'lucide-react'
import { MenuList, usePopover } from '../shell/popover'
import { candidateItem, type ToolItemModel } from './actions'
import { iconOf } from './icons'

export function ToolItem({ item, testId, size = 18, align = 'end' }: { item: ToolItemModel; testId?: string; size?: number; align?: 'start' | 'end' }) {
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const icon = typeof item.icon === 'string' ? item.icon : ''
  const menu = item.menu
  const disabled = item.reason !== null && !open
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-tool" aria-label={item.label} title={item.reason ?? item.tip} aria-pressed={item.pressed}
        aria-haspopup={menu ? 'menu' : undefined} aria-expanded={menu ? open : undefined} disabled={disabled}
        data-testid={testId} data-action={item.def.id} data-icon={icon} data-reason={item.reason ?? undefined}
        onClick={() => { if (menu) toggle(); else item.run?.() }}>
        {createElement(iconOf(item.icon) ?? Plus, { size, 'aria-hidden': true })}
      </button>
      {open && menu && <MenuList items={menu.map(candidateItem)} label={item.label} className={`aiws-popover aiws-popover-${align}`} testId={testId ? `${testId}-list` : undefined} onDone={close} />}
    </span>
  )
}
