import { useQuery } from '../lib'
import { dataModel } from '../model'
import { Badge, Empty, ErrorBox, Fields, Mono, Panel, Sid, Time } from '../ui'

export default function LoaderPage() {
  const { data, error } = useQuery(['loader.status'], () => dataModel.loaderStatus())
  if (!data) return error ? <ErrorBox error={error} /> : <Empty>Loading…</Empty>

  return (
    <>
      <ErrorBox error={error} />
      <Panel title="Loader" testId="loader-info">
        <Fields
          rows={[
            ['agent_did', <Mono>{data.agent_did}</Mono>],
            ['agent_id', <Mono>{data.agent_id}</Mono>],
            ['who', <Mono>{data.who}</Mono>],
            ['agent_root', <Mono>{data.agent_root}</Mono>],
            ['started', <Time ms={data.started_at_ms} />],
          ]}
        />
      </Panel>
      <Panel title="Modules" testId="loader-modules">
        <table className="tbl">
          <thead>
            <tr>
              <th>name</th>
              <th>enabled</th>
              <th>running</th>
              <th>note</th>
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
                  <Badge value={m.running ? 'running' : 'stopped'} tone={m.running ? 'ok' : m.enabled ? 'pending' : undefined} />
                </td>
                <td className="text-xs">{m.note}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </Panel>
      <Panel title={`Hosted sessions (${data.hosted.length})`} testId="loader-hosted">
        {data.hosted.length ? (
          <table className="tbl">
            <thead>
              <tr>
                <th>sid</th>
                <th>class</th>
                <th>loaded</th>
                <th>drives</th>
                <th>last result</th>
                <th>when</th>
                <th>idle unload</th>
                <th>reason</th>
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
                    <Badge value={h.loaded ? 'loaded' : 'unloaded'} tone={h.loaded ? 'ok' : undefined} />
                    {h.loaded && <span className="ml-1"><Time ms={h.loaded_at_ms} /></span>}
                  </td>
                  <td>{h.drives}</td>
                  <td title={JSON.stringify(h.last_result)}>
                    {h.last_result ? <Badge value={h.last_result.kind} tone={/error|failed|lost|blocked/.test(h.last_result.kind) ? 'bad' : undefined} /> : '–'}
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
          <Empty>No hosted session.</Empty>
        )}
      </Panel>
      <Panel title="UI inbox bridge" testId="loader-ui">
        {data.ui ? (
          <>
            <div className="mb-2 flex flex-wrap gap-4 text-xs">
              <span>scans: {data.ui.scans}</span>
              <span>
                last scan: <Time ms={data.ui.last_scan_ms} />
              </span>
              {data.ui.last_error && <span className="text-bad">last error: {data.ui.last_error}</span>}
            </div>
            {data.ui.inboxes.length ? (
              <table className="tbl">
                <thead>
                  <tr>
                    <th>route_key</th>
                    <th>session</th>
                    <th>delivered</th>
                    <th>dropped</th>
                    <th>held</th>
                    <th>last</th>
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
                      <td className="text-xs text-warn">{i.held}</td>
                      <td>
                        <Time ms={i.last_at_ms} />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <Empty>No inbox bound.</Empty>
            )}
          </>
        ) : (
          <Empty>The ui module is not enabled.</Empty>
        )}
      </Panel>
      <Panel title={`Recent errors (${data.errors.length})`} testId="loader-errors">
        {data.errors.length ? (
          <table className="tbl">
            <tbody>
              {[...data.errors].reverse().map((e, i) => (
                <tr key={`${e.at_ms}-${i}`}>
                  <td className="w-px">
                    <Time ms={e.at_ms} />
                  </td>
                  <td className="w-px">
                    <Mono>{e.source}</Mono>
                  </td>
                  <td className="text-xs text-bad">{e.message}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <Empty>No error recorded.</Empty>
        )}
      </Panel>
    </>
  )
}
