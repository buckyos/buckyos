/* What the framework keeps for the whole open Workspace, whatever the view (canvas, data mode, a show, a control's own
 * page): this window's presence registration (K2: the others see it from the moment the Workspace opens), and the
 * customize dialog asked for by "视图 ▸ 自定义工具栏…" or App mode's ⋯ (UI 框架 PRD §4.6). Mounted once by the shell. */

import { useEffect, useState } from 'react'
import { CustomizeDialog } from '../customize/CustomizeDialog'
import { onCustomizeRequest } from './appMode'
import { PresenceKeeper } from './members'

export function FrameworkHost() {
  const [customizing, setCustomizing] = useState(false)
  useEffect(() => onCustomizeRequest(() => setCustomizing(true)), [])
  return (
    <>
      <PresenceKeeper />
      {customizing && <CustomizeDialog onClose={() => setCustomizing(false)} />}
    </>
  )
}
