import { t } from '../i18n'
import { useState } from 'react'
import { useQuery } from '../lib'
import { dataModel, type MemoryQueryResult } from '../model'
import { Badge, Empty, ErrorBox, Fields, Mono, Panel, Pre, Sid, Time } from '../ui'

// The perception backlog is consolidated by the built-in self improvement Goal
// (see the Goals section of the home page); `perception.last_seq` is the only
// perception read left and it is per session.
function Perception() {
  return (
    <Panel title={t('observePerception')} testId="agent-perception">
      <Empty>{t('observePerceptionsConsolidated')}</Empty>
    </Panel>
  )
}

function ArtifactVersions({ aid }: { aid: string }) {
  const { data, error } = useQuery(['artifacts.versions', aid], () => dataModel.artifactVersions(aid))
  if (error) return <ErrorBox error={error} />
  if (!data) return <Empty>{t('loading')}</Empty>
  return data.length ? (
    <table className="tbl">
      <thead>
        <tr>
          <th>{t('observeVer')}</th>
          <th>{t('observeState')}</th>
          <th>{t('observeBase')}</th>
          <th>{t('observeSession')}</th>
          <th>{t('observeOutputs')}</th>
          <th>{t('observeSideEffects')}</th>
          <th>{t('observeUpdated')}</th>
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
    <Empty>{t('observeNoVersions')}</Empty>
  )
}

function Artifacts() {
  const { data, error } = useQuery(['artifacts.list'], () => dataModel.artifacts())
  const [open, setOpen] = useState('')
  return (
    <Panel title={t('observeArtifacts')} count={data?.length ?? 0} testId="agent-artifacts">
      <ErrorBox error={error} />
      {data?.length ? (
        <table className="tbl">
          <thead>
            <tr>
              <th>aid</th>
              <th>{t('observeHead')}</th>
              <th>{t('observeRev')}</th>
              <th>{t('observeUpdated')}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {data.map((a) => (
              <tr key={a.aid}>
                <td className="whitespace-nowrap"><Mono>{a.aid}</Mono></td>
                <td><Mono>{a.head ?? '–'}</Mono></td>
                <td>{a.rev}</td>
                <td><Time ms={a.updated_at_ms} /></td>
                <td className="w-full">
                  <button className="btn" aria-expanded={open === a.aid} onClick={() => setOpen(open === a.aid ? '' : a.aid)}>
                    {open === a.aid ? t('skDraftHide') : t('observeVersions')}
                  </button>
                  {open === a.aid && <div className="mt-2 rounded-lg bg-bg px-3 py-1"><ArtifactVersions aid={a.aid} /></div>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : (
        <Empty>{t('observeNoArtifacts')}</Empty>
      )}
    </Panel>
  )
}

function BehaviorConfig({ name }: { name: string }) {
  const { data, error } = useQuery(['behaviors.get', name], () => dataModel.behavior(name), false)
  if (error) return <ErrorBox error={error} />
  return data === undefined ? <Empty>{t('loading')}</Empty> : <Pre value={data ?? t('observeNotFound')} />
}

function Behaviors() {
  const { data, error } = useQuery(['behaviors'], () => dataModel.behaviors())
  const identity = useQuery(['behaviors.identity'], () => dataModel.identity())
  const [open, setOpen] = useState('')
  return (
    <>
      <Panel
        title={t('observeBehaviors')}
        count={data?.behaviors.length ?? 0}
        testId="agent-behaviors"
        extra={data && <span>{t('observeRevision')} <Mono>{data.revision}</Mono></span>}
      >
        <ErrorBox error={error} />
        {data?.behaviors.length ? (
          <table className="tbl">
            <thead>
              <tr>
                <th>{t('observeName')}</th>
                <th>{t('observeObjective')}</th>
                <th>{t('observeNext')}</th>
              </tr>
            </thead>
            <tbody>
              {data.behaviors.map((b) => (
                <tr key={b.name}>
                  <td>
                    <button className="whitespace-nowrap font-mono text-xs text-accent hover:underline" aria-expanded={open === b.name} onClick={() => setOpen(open === b.name ? '' : b.name)}>
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
          <Empty>{t('observeNoBehaviors')}</Empty>
        )}
        {open && (
          <div className="mt-3" data-testid="behavior-config">
            <div className="mb-2 text-xs text-mute">behaviors.get <Mono>{open}</Mono></div>
            <BehaviorConfig name={open} />
          </div>
        )}
      </Panel>
      <Panel title={t('observeIdentity')} testId="agent-identity">
        <ErrorBox error={identity.error} />
        {identity.data?.role || identity.data?.self ? (
          <Fields
            rows={[
              [t('observeRole'), identity.data.role ? <Pre value={identity.data.role} /> : null],
              [t('observeSelf'), identity.data.self ? <Pre value={identity.data.self} /> : null],
              ['i18n', identity.data.i18n ? <Mono>{Object.keys(identity.data.i18n).join(', ')}</Mono> : null],
            ]}
          />
        ) : (
          <Empty>{t('observeNoIdentityText')}</Empty>
        )}
      </Panel>
    </>
  )
}

function Hints() {
  const [session, setSession] = useState('')
  const [words, setWords] = useState('')
  const [result, setResult] = useState<MemoryQueryResult | null>(null)
  const [needSession, setNeedSession] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const recall = async () => {
    setError(null)
    // Memory answers as one session: its grants decide what is visible.
    if (!session.trim()) {
      setNeedSession(true)
      setResult(null)
      return
    }
    setNeedSession(false)
    try {
      setResult(await dataModel.queryMemory(session.trim(), words.split(/[\s,]+/).filter(Boolean)))
    } catch (e) {
      setError(e)
    }
  }
  const rows = result ? [...result.cognitions, ...result.recent_perceptions] : []
  return (
    <Panel title={t('observeMemoryRecallAsASession')} testId="agent-hints">
      <form
        className="mb-3 flex flex-wrap gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          void recall()
        }}
      >
        <input className="input w-48 font-mono" placeholder={t('observeSessionId')} aria-label={t('goalSession')} value={session} onChange={(e) => setSession(e.target.value)} />
        <input className="input min-w-[12rem] flex-1" placeholder={t('observeWordsOrTagsSpaceOrCommaSeparated')} aria-label={t('observeTags')} value={words} onChange={(e) => setWords(e.target.value)} />
        <button className="btn btn-primary" type="submit">{t('observeRecall')}</button>
      </form>
      <ErrorBox error={error} />
      {needSession && <Empty>{t('observeMemoryAnswersAsOneSessionEnterItsId')}</Empty>}
      {result &&
        (rows.length ? (
          <table className="tbl">
            <tbody>
              {rows.map((h) => (
                <tr key={h.reference}>
                  <td className="w-px whitespace-nowrap"><Mono>{h.reference}</Mono></td>
                  <td className="w-px">
                    <Badge value={h.layer === 'cognition' ? h.state : t('observeNotYetConsolidated')} />
                  </td>
                  <td className="text-xs">{h.summary}</td>
                  <td className="w-px whitespace-nowrap text-xs text-mute">{h.observed_at}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <Empty>{result.status === 'not_triggered' ? 'Nothing to look up by.' : 'No hints.'}</Empty>
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
