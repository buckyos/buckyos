import type { ShareDialogParams } from '../../sysdlg/ShareDialog'
import { cyfsPathToLocal } from '../../components/preview/session'
import { isCyfsPathRef, type ContentRef } from '../../components/preview/types'

/**
 * What the system share dialog (`sysdlg/share`) takes for a piece of content: the DFS path of a file of this Zone.
 * Content known only by object id, a bucket key, a chat attachment or a draft blob has none yet, nor has a pinned
 * version of a file (the dialog shares what the path holds now): null, and the caller says why.
 */
export function shareParamsOf(source: ContentRef, title: string): ShareDialogParams | null {
  if (!isCyfsPathRef(source) || source.version) return null
  if (/^(cyfs|dfs):\/\/[^/]/.test(source.path.trim())) return null
  const path = cyfsPathToLocal(source.path)
  return path === '/' ? null : { source: path, title }
}
