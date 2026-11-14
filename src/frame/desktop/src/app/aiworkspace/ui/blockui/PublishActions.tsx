/* A control's body offers its actions to the Block menu (UI 框架 PRD §10.1) by rendering this while it is mounted. */

import { publishedOf, usePublishActions } from './commands'

type Item = { id: string; label: string; checked?: boolean; disabled?: string | false; run: () => void } | 'separator'

export function PublishActions({ containerId, items }: { containerId: string; items: readonly Item[] }) {
  usePublishActions(containerId, publishedOf(items))
  return null
}
