/* The browser of an Agent is ext-core's online item `chromium`, downloaded by each node. For an administrator, where
 * that download stands on the nodes (control-panel `ext_tools.status`) and a button that starts it; once it is there the
 * App connects without being reloaded. */

import { useCallback, useEffect, useRef, useState } from 'react'
import { downloadExtToolsOnline, fetchExtToolsStatus, type ExtToolsStatusResponse } from '../../../api/settings'
import { useI18n } from '../../../i18n/provider'
import { formatBytes } from '../../settings/components/shared/format'
import { ONLINE_POLL_MS, onlineBusy, onlineErrorText, onlineRequest, onlineStateLabel, packageOfItem, requestPending, type OnlineRequest } from '../../settings/pages/extToolsOnline'
import { OnlineProgress } from '../../settings/pages/OnlineProgress'
import { useAgentBrowserStore } from '../store/context'
import './download.css'

const ITEM = 'chromium'
const PACKAGE = 'ext-core'

export function BrowserDownload() {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const [status, setStatus] = useState<ExtToolsStatusResponse | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const [requests, setRequests] = useState<Record<string, OnlineRequest>>({})
  const [busy, setBusy] = useState(false)
  const wasReady = useRef(false)

  const load = useCallback(async () => {
    const { data, error } = await fetchExtToolsStatus()
    if (data) setStatus(data)
    setFailure(error ? String(error) : null)
  }, [])

  const pkg = packageOfItem(status, ITEM)
  const nodes = Object.entries(pkg?.nodes ?? {}).flatMap(([node, report]) => (report.online?.[ITEM] ? [{ node, status: report.online[ITEM] }] : []))
  const following = nodes.some(({ node, status: item }) => onlineBusy(item, requests[node]))
  const ready = nodes.length > 0 && nodes.every(({ status: item }) => item.state === 'ready')
  const startable = nodes.filter(({ node, status: item }) => !onlineBusy(item, requests[node]) && (item.state === 'missing' || item.state === 'failed'))

  useEffect(() => {
    const first = window.setTimeout(() => void load(), 0)
    return () => window.clearTimeout(first)
  }, [load])

  // The Agent's own sentence about the download is kept as fresh as the progress.
  useEffect(() => {
    if (!following) return
    const timer = window.setInterval(() => {
      void load()
      void store.refreshStatus()
    }, ONLINE_POLL_MS)
    return () => window.clearInterval(timer)
  }, [load, following, store])

  // The browser is there: look at once instead of at the App's next `browser.status`.
  useEffect(() => {
    if (ready && !wasReady.current) void store.refreshStatus()
    wasReady.current = ready
  }, [ready, store])

  const download = async () => {
    setBusy(true)
    const asked = await Promise.all(startable.map(({ node }) => downloadExtToolsOnline(pkg?.id ?? PACKAGE, ITEM, node)))
    setBusy(false)
    const refused = asked.find((result) => result.error)
    setFailure(refused ? String(refused.error) : null)
    setRequests((current) => ({ ...current, ...Object.fromEntries(startable.map(({ node, status: item }) => [node, onlineRequest(item)])) }))
    void load()
  }

  if (!status) return failure ? <p className="ab-download-error" data-testid="ab-download-failure">{failure}</p> : null
  const size = nodes.find(({ status: item }) => item.size_bytes > 0)?.status.size_bytes
  return (
    <div className="ab-download" data-testid="ab-download">
      {nodes.map(({ node, status: item }) => {
        const pending = requestPending(requests[node], item)
        const problem = onlineErrorText(item, t)
        return (
          <div key={node} className="ab-download-node" data-testid={`ab-download-${node}`} data-state={pending ? 'requested' : item.state}>
            <p className="ab-download-state">
              {nodes.length > 1 ? `${node}: ` : ''}
              {item.state === 'ready' && !pending ? t('agentBrowser.download.ready', 'Downloaded. Connecting…') : onlineStateLabel(item, pending, t)}
            </p>
            {item.state === 'downloading' ? <OnlineProgress status={item} /> : null}
            {problem ? <p className="ab-download-error">{problem}</p> : null}
          </div>
        )
      })}
      {startable.length > 0 ? (
        <>
          <button type="button" className="ab-button ab-button-primary" data-testid="ab-download-browser" disabled={busy} onClick={() => void download()}>
            {startable.some(({ status: item }) => item.state === 'failed') ? t('agentBrowser.download.retry', 'Retry the download') : t('agentBrowser.download.start', 'Download the browser')}
          </button>
          {size ? <p className="ab-download-hint">{t('agentBrowser.download.hint', 'It is downloaded once from its official source ({{size}}) and shared by every Agent on the node.', { size: formatBytes(size) })}</p> : null}
        </>
      ) : null}
      {failure ? <p className="ab-download-error" data-testid="ab-download-failure">{failure}</p> : null}
    </div>
  )
}
