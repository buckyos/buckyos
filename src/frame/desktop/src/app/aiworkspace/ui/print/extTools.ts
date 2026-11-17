/* "设置 › 扩展工具" from the print dialog (the browser, the font package): a window of this Desktop, or the Desktop of
 * the Zone host in a tab of its own from a Workspace's own origin (it has no Desktop windows). */

import { openSettings } from '../../../settings/launch'
import { onWorkspaceHost, zoneOrigin } from '../../links'

export function openExtTools() {
  if (onWorkspaceHost()) window.open(`${zoneOrigin()}/?settings=ext-tools`, '_blank', 'noopener')
  else void openSettings('ext-tools')
}
