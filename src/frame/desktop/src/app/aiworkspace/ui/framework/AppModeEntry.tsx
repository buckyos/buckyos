/* App mode's collapsed ⋯, top left (UI 框架 PRD §4.6): only for whoever may change the layout (`structure`, `manage`);
 * it shows the hidden zones in this window for a while, or opens "自定义工具栏…" (not on a phone, §6.1). Everyone else
 * gets nothing here — the canvas's own controls are the way in, and the window's or the browser's back and close the way
 * out. Alerts stay above the status bar either way. */

import { Ellipsis } from 'lucide-react'
import { useOutlineVersion, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { MenuButton, type MenuItem } from '../shell/popover'
import { useShell } from '../shell/shellContext'
import { mayCustomize, requestCustomize, setZonesRevealed } from './appMode'
import './framework.css'

export function AppModeEntry() {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  useOutlineVersion()
  if (shell.identity.guest || !mayCustomize(store, shell.activeSurface?.entity_id ?? null)) return null
  const items: MenuItem[] = [
    { id: 'app-mode-reveal', label: t('aiworkspace.customize.reveal'), run: () => setZonesRevealed(store, true) },
    ...(shell.phone ? [] : [{ id: 'app-mode-customize', label: t('aiworkspace.customize.menu'), run: requestCustomize }]),
  ]
  return (
    <div className="aiws-app-mode-entry">
      <MenuButton label={t('aiworkspace.customize.entry')} title={t('aiworkspace.customize.entry')} className="aiws-tool aiws-app-mode-button" testId="aiws-app-mode-entry" menuTestId="aiws-app-mode-menu" items={items}>
        <Ellipsis size={18} />
      </MenuButton>
    </div>
  )
}
