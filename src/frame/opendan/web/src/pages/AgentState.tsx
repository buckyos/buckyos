import { useState } from 'react'
import { useQuery } from '../lib'
import { dataModel, type Hint } from '../model'
import { Badge, Empty, ErrorBox, Fields, Mono, Panel, Pre, Sid, Time } from '../ui'

// The perception backlog is consolidated by the built-in memory maintenance
// Goal (see the Goals section of the home page); `perception.last_seq` is the
// only perception read left and it is per session.
function Perception() {
  return (
    <Panel title="Perception" testId="agent-perception">
      <Empty>Perceptions are consolidated by the memory maintenance goal; see Goals on the home page.</Empty>
    </Panel>
  )
}

function ArtifactVersions({ aid }: { aid: string }) {
  const { data, error } = useQuery(['artifacts.versions', aid], () => dataModel.artifactVersions(aid))
  if (error) return <ErrorBox error={error} />
  if (!data) return <Empty>Loading…</Empty>
  return data.length ? (
    <table className="tbl">
      <thead>
        <tr>
          <th>ver</th>
          <th>state</th>
          <th>base</th>
          <th>session</th>
          <th>outputs</th>
          <th>side effects</th>
          <th>updated</th>
        </tr>
      </thead>
      <tbody>
        {data.map((v) => (
          <tr key={v.ver}>
            <td><Mono>{v.ver}</Mono></td>
            <td><Badge value={v.state} tone={v.state === 'produced' ? 'pending' : v.state} /></td>
            <td><Mono>{v.base ?? '–'}</Mono></td>
            <td><Sid sid={v.session} /></td>
            <td><Mono>{(v.outputs ?? []).join(', ')}</Mono></td>
            <td className="text-xs">{(v.side_effects ?? []).map((s) => `${s.tool}: ${s.note}`).join('; ')}</td>
            <td><Time ms={v.updated_at_ms} /></td>
          </tr>
        ))}
      </tbody>
    </table>
  ) : (
    <Empty>No versions.</Empty>
  )
}

function Artifacts() {
  const { data, error } = useQuery(['artifacts.list'], () => dataModel.artifacts())
  const [open, setOpen] = useState('')
  return (
    <Panel title={`Artifacts (${data?.length ?? 0})`} testId="agent-artifacts">
      <ErrorBox error={error} />
      {data?.length ? (
        <table className="tbl">
          <thead>
            <tr>
              <th>aid</th>
              <th>head</th>
              <th>rev</th>
              <th>updated</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {data.map((a) => (
              <tr key={a.aid}>
                <td><Mono>{a.aid}</Mono></td>
                <td><Mono>{a.head ?? '–'}</Mono></td>
                <td>{a.rev}</td>
                <td><Time ms={a.updated_at_ms} /></td>
                <td className="w-full">
                  <button className="btn" onClick={() => setOpen(open === a.aid ? '' : a.aid)}>
                    {open === a.aid ? 'Hide' : 'Versions'}
                  </button>
                  {open === a.aid && <div className="mt-1"><ArtifactVersions aid={a.aid} /></div>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : (
        <Empty>No artifacts.</Empty>
      )}
    </Panel>
  )
}

function BehaviorConfig({ name }: { name: string }) {
  const { data, error } = useQuery(['behaviors.get', name], () => dataModel.behavior(name), false)
  if (error) return <ErrorBox error={error} />
  return data === undefined ? <Empty>Loading…</Empty> : <Pre value={data ?? 'not found'} />
}

function Behaviors() {
  const { data, error } = useQuery(['behaviors'], () => dataModel.behaviors())
  const identity = useQuery(['behaviors.identity'], () => dataModel.identity())
  const [open, setOpen] = useState('')
  return (
    <>
      <Panel
        title={`Behaviors (${data?.behaviors.length ?? 0})`}
        testId="agent-behaviors"
        extra={data && <span className="text-xs text-mute">revision <Mono>{data.revision}</Mono></span>}
      >
        <ErrorBox error={error} />
        {data?.behaviors.length ? (
          <table className="tbl">
            <thead>
              <tr>
                <th>name</th>
                <th>objective</th>
                <th>next</th>
              </tr>
            </thead>
            <tbody>
              {data.behaviors.map((b) => (
                <tr key={b.name}>
                  <td>
                    <button className="font-mono text-xs text-accent hover:underline" onClick={() => setOpen(open === b.name ? '' : b.name)}>
                      {b.name}
                    </button>
                  </td>
                  <td className="text-xs">{b.objective}</td>
                  <td><Mono>{(b.next ?? []).join(', ')}</Mono></td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <Empty>No behaviors.</Empty>
        )}
        {open && (
          <div className="mt-2" data-testid="behavior-config">
            <div className="mb-1 text-xs text-mute">behaviors.get <Mono>{open}</Mono></div>
            <BehaviorConfig name={open} />
          </div>
        )}
      </Panel>
      <Panel title="Identity" testId="agent-identity">
        <ErrorBox error={identity.error} />
        {identity.data?.role || identity.data?.self ? (
          <Fields
            rows={[
              ['role', identity.data.role ? <Pre value={identity.data.role} /> : null],
              ['self', identity.data.self ? <Pre value={identity.data.self} /> : null],
              ['i18n', identity.data.i18n ? <Mono>{Object.keys(identity.data.i18n).join(', ')}</Mono> : null],
            ]}
          />
        ) : (
          <Empty>No identity text.</Empty>
        )}
      </Panel>
    </>
  )
}

function Hints() {
  const [tags, setTags] = useState('')
  const [hints, setHints] = useState<Hint[] | null>(null)
  const [error, setError] = useState<unknown>(null)
  const recall = async () => {
    setError(null)
    try {
      setHints(await dataModel.recallHints(tags.split(/[\s,]+/).filter(Boolean)))
    } catch (e) {
      setError(e)
    }
  }
  return (
    <Panel title="Hints recall" testId="agent-hints">
      <form
        className="mb-2 flex gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          void recall()
        }}
      >
        <input className="input flex-1" placeholder="tags (space or comma separated)" aria-label="Tags" value={tags} onChange={(e) => setTags(e.target.value)} />
        <button className="btn" type="submit">Recall</button>
      </form>
      <ErrorBox error={error} />
      {hints &&
        (hints.length ? (
          <table className="tbl">
            <tbody>
              {hints.map((h) => (
                <tr key={h.id}>
                  <td className="w-px"><Mono>{h.id}</Mono></td>
                  <td className="w-px">{h.kind && <Badge value={h.kind} />}</td>
                  <td className="text-xs">{h.sentence}</td>
                  <td className="w-px whitespace-nowrap text-xs text-mute">{h.time}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <Empty>No hints.</Empty>
        ))}
    </Panel>
  )
}

export default function AgentStatePage() {
  return (
    <>
      <Perception />
      <Artifacts />
      <Behaviors />
      <Hints />
    </>
  )
}
