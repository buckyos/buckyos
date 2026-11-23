import { t } from '../i18n'
import { useQuery } from '../lib'
import { dataModel } from '../model'
import { Badge, Dot, Empty, ErrorBox, Fields, Mono, Panel, Sid, Time } from '../ui'

export default function LoaderPage() {
  const { data, error } = useQuery(['loader.status'], () => dataModel.loaderStatus())
  if (!data) return error ? <ErrorBox error={error} /> : <Empty>{t('loading')}</Empty>

  return (
    <>
      <ErrorBox error={error} />
      <Panel title={t('observeLoader')} testId="loader-info">
        <Fields
          rows={[
            [t('observeAgentDid'), <Mono>{data.agent_did}</Mono>],
            [t('observeAgentId'), <Mono>{data.agent_id}</Mono>],
            [t('observeWho'), <Mono>{data.who}</Mono>],
            [t('observeAgentRoot'), <Mono>{data.agent_root}</Mono>],
            [t('observeStarted'), <Time ms={data.started_at_ms} />],
          ]}
        />
      </Panel>
      <Panel title={t('observeAuthentication')} testId="loader-authentication">
        {data.authentication ? (
          <Fields rows={[
            [t('observeState'), <Badge value={data.authentication.state} tone={data.authentication.state === 'ready' ? 'ok' : data.authentication.state === 'retry_pending' ? 'pending' : 'bad'} />],
            [t('observeError'), data.authentication.last_error ?? '–'],
            [t('observeCode'), data.authentication.error_code ?? '–'],
            [t('gateSince'), data.authentication.since ? <Time ms={data.authentication.since * 1000} /> : '–'],
            [t('observeNextRetry'), data.authentication.next_retry_time ? <Time ms={data.authentication.next_retry_time * 1000} /> : '–'],
          ]} />
        ) : <Empty>{t('observeAuthenticationIsNotConfiguredInDevelopmentMode')}</Empty>}
      </Panel>
      <Panel title={t('observeModules')} testId="loader-modules">
        <table className="tbl">
          <thead>
            <tr>
              <th>{t('observeName')}</th>
              <th>{t('observeEnabled')}</th>
              <th>{t('observeRunning')}</th>
              <th>{t('observeNote')}</th>
            </tr>
          </thead>
          <tbody>
            {data.modules.map((m) => (
              <tr key={m.name}>
                <td>
                  <Mono>{m.name}</Mono>
                </td>
                <td>
                  <Badge value={m.enabled ? 'enabled' : 'disabled'} tone={m.enabled ? 'ok' : undefined} />
                </td>
                <td>
                  <Badge value={m.running ? t('observeRunning') : t('observeStopped')} tone={m.running ? 'ok' : m.enabled ? 'pending' : undefined} />
                </td>
                <td className="text-xs">{m.note}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </Panel>
      <Panel title={t('observeHostedSessions')} count={data.hosted.length} testId="loader-hosted">
        {data.hosted.length ? (
          <table className="tbl">
            <thead>
              <tr>
                <th>sid</th>
                <th>{t('observeClass')}</th>
                <th>{t('observeLoaded')}</th>
                <th>{t('observeDrives')}</th>
                <th>{t('observeLastResult')}</th>
                <th>{t('observeWhen')}</th>
                <th>{t('observeIdleUnload')}</th>
                <th>{t('observeReason')}</th>
              </tr>
            </thead>
            <tbody>
              {data.hosted.map((h) => (
                <tr key={h.session_id}>
                  <td>
                    <Sid sid={h.session_id} />
                  </td>
                  <td>{h.class}</td>
                  <td>
                    <Badge value={h.loaded ? t('observeLoaded') : t('observeUnloaded')} tone={h.loaded ? 'ok' : undefined} />
                    {h.loaded && <span className="ml-2"><Time ms={h.loaded_at_ms} /></span>}
                  </td>
                  <td>{h.drives}</td>
                  <td title={JSON.stringify(h.last_result)}>
                    {h.last_result ? <Badge value={h.last_result.kind} tone={/error|failed|lost|blocked/.test(h.last_result.kind) ? 'bad' : undefined} /> : <span className="text-mute">–</span>}
                  </td>
                  <td>
                    <Time ms={h.last_result_at_ms} />
                  </td>
                  <td>{h.idle_unload_secs == null ? 'never' : `${h.idle_unload_secs}s`}</td>
                  <td className="text-xs">{h.reason}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <Empty>{t('observeNoHostedSession')}</Empty>
        )}
      </Panel>
      <Panel title={t('observeUiInboxBridge')} testId="loader-ui">
        {data.ui ? (
          <>
            <div className="mb-3 flex flex-wrap items-center gap-x-6 gap-y-1 text-xs">
              <span><span className="text-mute">{t('observeScans')}</span> <span className="tabular-nums">{data.ui.scans}</span></span>
              <span>
                <span className="text-mute">{t('observeLastScan')}</span> <Time ms={data.ui.last_scan_ms} />
              </span>
              {data.ui.last_error && <span className="inline-flex items-baseline gap-2 text-bad"><Badge value={t('observeLastError')} tone="bad" /><span className="text-fg">{data.ui.last_error}</span></span>}
            </div>
            {data.ui.inboxes.length ? (
              <table className="tbl">
                <thead>
                  <tr>
                    <th>route_key</th>
                    <th>{t('observeSession')}</th>
                    <th>{t('observeDelivered')}</th>
                    <th>{t('observeDropped')}</th>
                    <th>{t('observeHeld')}</th>
                    <th>{t('observeLast')}</th>
                  </tr>
                </thead>
                <tbody>
                  {data.ui.inboxes.map((i) => (
                    <tr key={i.route_key}>
                      <td>
                        <Mono>{i.route_key}</Mono>
                      </td>
                      <td>{i.session_id ? <Sid sid={i.session_id} /> : '–'}</td>
                      <td>{i.delivered}</td>
                      <td className={i.dropped ? 'text-bad' : ''}>{i.dropped}</td>
                      <td className="text-xs">{i.held ? <Badge value={i.held} tone="waiting" /> : null}</td>
                      <td>
                        <Time ms={i.last_at_ms} />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <Empty>{t('observeNoInboxBound')}</Empty>
            )}
          </>
        ) : (
          <Empty>{t('observeTheUiModuleIsNotEnabled')}</Empty>
        )}
      </Panel>
      <Panel title={t('observeRecentErrors')} count={data.errors.length} testId="loader-errors">
        {data.errors.length ? (
          <table className="tbl">
            <tbody>
              {[...data.errors].reverse().map((e, i) => (
                <tr key={`${e.at_ms}-${i}`}>
                  <td className="w-px">
                    <Time ms={e.at_ms} />
                  </td>
                  <td className="w-px whitespace-nowrap">
                    <Mono className="text-mute">{e.source}</Mono>
                  </td>
                  <td className="text-xs">
                    <span className="flex items-start gap-2"><Dot tone="bad" className="mt-[7px]" />{e.message}</span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <Empty>{t('observeNoErrorRecorded')}</Empty>
        )}
      </Panel>
    </>
  )
}
