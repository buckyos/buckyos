/* The right panel (UI improvement §3.2): one kind of content at a time — properties, references and
 * dependencies, annotations, collaboration, save state — switched by tabs, never several narrow
 * columns. In a wide window it takes real layout width (the canvas and its camera get the rest); in a
 * narrow one it is a drawer over the canvas, so closing it restores the original viewport. */

import type { ReactNode } from 'react'
import { X } from 'lucide-react'
import { SIDE_TAB_LABEL, useShell, type SideTab } from './shellContext'

export function SidePanel({ tabs, render }: { tabs: SideTab[]; render: (tab: SideTab) => ReactNode }) {
  const shell = useShell()
  const tab = shell.side
  if (!tab || !tabs.includes(tab)) return null
  const drawer = shell.size !== 'wide'
  return (
    <aside className={`aiws-side${drawer ? ' is-drawer' : ''}`} aria-label={SIDE_TAB_LABEL[tab]} data-testid="aiws-side-panel" data-tab={tab}>
      <div className="aiws-side-head">
        <div className="aiws-tabs" role="tablist" aria-label="面板内容">
          {tabs.map((t) => <button key={t} type="button" role="tab" aria-selected={t === tab} data-testid={`aiws-side-tab-${t}`} onClick={() => shell.setSide(t)}>{SIDE_TAB_LABEL[t]}</button>)}
        </div>
        <button type="button" className="aiws-tool aiws-tool-small" aria-label="关闭面板" title="关闭面板" data-testid="aiws-side-close" onClick={() => shell.setSide(null)}><X size={16} /></button>
      </div>
      <div className="aiws-side-body" data-testid="aiws-canvas-side">{render(tab)}</div>
    </aside>
  )
}
