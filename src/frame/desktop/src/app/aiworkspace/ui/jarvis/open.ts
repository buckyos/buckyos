/* Opening the Agent's panel (JarvisPanel.tsx): its state for the open workspace, and the action of "协作 ▸ 与 Agent 对话"
 * and the button beside the status bubble. */

import { Bot } from 'lucide-react'
import { aiwsT } from '../../i18n'
import { Emitter } from '../../state/emitter'
import { actionRegistry } from '../framework/actions'
import type { ShellApi } from '../shell/shellContext'

export const jarvisPanel = { open: false, emitter: new Emitter() }

/** Open (or close) the Agent's panel of the open workspace. */
export function openJarvis(open = true) {
  if (jarvisPanel.open === open) return
  jarvisPanel.open = open
  jarvisPanel.emitter.emit()
}

/** Guests and restricted sessions have no Agent conversation of their own. */
export function canChatWithAgent(identity: ShellApi['identity']) {
  return !identity.guest && !identity.restricted
}

actionRegistry.register({
  id: 'collab.jarvis', label: () => aiwsT('aiworkspace.jarvis.action'), tip: () => aiwsT('aiworkspace.jarvis.actionHint'), icon: Bot, phone: true,
  menu: { group: 'collab', order: 70, id: 'jarvis', separator: true },
  visible: ({ shell }) => canChatWithAgent(shell.identity),
  run: () => openJarvis(),
})
