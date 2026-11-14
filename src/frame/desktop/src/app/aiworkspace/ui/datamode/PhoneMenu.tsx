/* The phone's main menu in the data mode's top bar (UI 框架 PRD §5.6, §6.1): what the phone may use of the main menu
 * (`ActionDef.phone`), as a bottom panel entered one group at a time — "返回画布" among it. */

import { useState } from 'react'
import { Ellipsis } from 'lucide-react'
import { buildMenu } from '../framework/actions'
import { useActionContext } from '../framework/hooks'
import type { CanvasCommands } from '../shell/MainMenu'
import { useAiwsText } from '../../useAiwsText'
import { SheetMenu } from './menus'

export function PhoneMainMenu({ canvas }: { canvas: CanvasCommands | null }) {
  const { t } = useAiwsText()
  const ctx = useActionContext(canvas)
  const [open, setOpen] = useState(false)
  return (
    <>
      <button type="button" className="aiws-tool" aria-label={t('aiworkspace.shell.toolbar.mainMenu')} title={t('aiworkspace.shell.toolbar.mainMenu')} aria-haspopup="menu" aria-expanded={open}
        data-testid="aiws-main-menu" onClick={() => setOpen(true)}><Ellipsis size={20} /></button>
      {open && <SheetMenu items={buildMenu(ctx, { phone: true })} label={t('aiworkspace.shell.toolbar.mainMenu')} testId="aiws-main-menu-list" onClose={() => setOpen(false)} />}
    </>
  )
}
