/* Opening the Agent's panel (JarvisPanel.tsx): its state for the open workspace, and the action of "协作 ▸ 与 Agent 对话"
 * and the collaboration zone. */

import { Bot } from 'lucide-react'
import { aiwsT } from '../../i18n'
import { Emitter } from '../../state/emitter'
import { actionRegistry } from '../framework/actions'

export const jarvisPanel = { open: false, emitter: new Emitter() }

/** Open (or close) the Agent's panel of the open workspace. */
export function openJarvis(open = true) {
  if (jarvisPanel.open === open) return
  jarvisPanel.open = open
  jarvisPanel.emitter.emit()
}

actionRegistry.register({
  id: 'collab.jarvis', label: () => aiwsT('aiworkspace.jarvis.action'), tip: () => aiwsT('aiworkspace.jarvis.actionHint'), icon: Bot, phone: true,
  menu: { group: 'collab', order: 70, id: 'jarvis', separator: true }, zone: { zone: 'collab', order: 35, testId: 'aiws-collab-jarvis' },
  // a guest has no Agent; a restricted session (External DID, Limited account) has no conversations of its own here
  visible: ({ shell }) => !shell.identity.guest && !shell.identity.restricted,
  run: () => openJarvis(),
})
