/* The right panel (UI improvement §3.2): one kind of content at a time — properties, references and
 * dependencies, annotations, collaboration, save state — switched by tabs, never several narrow
 * columns. In a wide window it takes real layout width (the canvas and its camera get the rest); in a
 * narrow one it is a drawer over the canvas, so closing it restores the original viewport. */

import type { ReactNode } from 'react'
import { X } from 'lucide-react'
import { useAiwsText } from '../../useAiwsText'
import { SIDE_TAB_LABEL, useShell, type SideTab } from './shellContext'

export function SidePanel({ tabs, render }: { tabs: SideTab[]; render: (tab: SideTab) => ReactNode }) {
  const shell = useShell()
  const { t } = useAiwsText()
  const tab = shell.side
  if (!tab || !tabs.includes(tab)) return null
  const drawer = shell.size !== 'wide' || shell.phone
  return (
    <aside className={`aiws-side${drawer ? ' is-drawer' : ''}`} aria-label={SIDE_TAB_LABEL[tab]} data-testid="aiws-side-panel" data-tab={tab}>
      <div className="aiws-side-head">
        <div className="aiws-tabs" role="tablist" aria-label={t('aiworkspace.shell.side.tabs')}>
          {tabs.map((item) => <button key={item} type="button" role="tab" aria-selected={item === tab} data-testid={`aiws-side-tab-${item}`} onClick={() => shell.setSide(item)}>{SIDE_TAB_LABEL[item]}</button>)}
        </div>
        <button type="button" className="aiws-tool aiws-tool-small" aria-label={t('aiworkspace.shell.side.close')} title={t('aiworkspace.shell.side.close')} data-testid="aiws-side-close" onClick={() => shell.setSide(null)}><X size={16} /></button>
      </div>
      <div className="aiws-side-body" data-testid="aiws-canvas-side">{render(tab)}</div>
    </aside>
  )
}
