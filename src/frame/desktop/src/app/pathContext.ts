/* ── Which page an app shows (`OpenAppRequest.path`) ──
 *
 * A window of the desktop keeps it, so a request for the page already shown only brings the window to the front; a
 * page that shows one app alone puts it in its address.
 */

import { createContext, useContext, useEffect } from 'react'

export const AppPathContext = createContext<(path: string) => void>(() => {})

export function useAppPath(path: string) {
  const report = useContext(AppPathContext)
  useEffect(() => report(path), [report, path])
}
