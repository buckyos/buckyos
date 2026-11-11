import { useState, type ReactNode } from 'react'
import clsx from 'clsx'
import { BookOpen, ChevronRight } from 'lucide-react'
import { useSWRConfig } from 'swr'
import { ago, t, type TextKey } from '../i18n'
import { sessionHref, toText, useQuery } from '../lib'
import {
  dataModel,
  type InstallPlan,
  type KnownRuntime,
  type SkillCommandsStatus,
  type SkillDraft,
  type SkillFinding,
  type SkillKind,
  type SkillMeta,
  type SkillRuntimeStatus,
  type SkillSource,
} from '../model'
import { pickFile } from '../pickFile'
import { readSkillPikg } from '../pikg'
import { Badge, ConfirmDialog, Empty, ErrorBox, Mono, Panel, Pre, Sid, Time, type ConfirmRequest } from '../ui'

const PRIVATE = 'private'
const KINDS: SkillKind[] = ['work', 'plan', 'goal']
const KIND_LABEL: Record<SkillKind, TextKey> = { work: 'skKindWork', plan: 'skKindPlan', goal: 'skKindGoal' }
const ORIGIN_LABEL: Record<SkillMeta['origin'], TextKey> = {
  package: 'skOriginPackage',
  installed: 'skOriginInstalled',
  imported: 'skOriginImported',
  local: 'skOriginLocal',
}
const FINDING_TONE: Record<SkillFinding['level'], string> = { error: 'bad', block: 'bad', high: 'bad', medium: 'waiting', low: 'finished' }
const RISK_LABEL: Record<InstallPlan['risk']['level'], TextKey> = { low: 'skRiskLow', medium: 'skRiskMedium', high: 'skRiskHigh' }

const fmtBytes = (n: number): string => (n < 1024 ? `${n} B` : `${(n / 1024).toFixed(1)} KB`)
const hasUi = (scope?: string) => Boolean(scope?.split(',').some((part) => part.trim() === 'ui'))
const isSkillsKey = (key: unknown) => Array.isArray(key) && typeof key[0] === 'string' && key[0].startsWith('skills.')

/** Reloads everything this page shows; the runtime probe included. */
function useSkillsRefresh() {
  const { mutate } = useSWRConfig()
  return () => void mutate(isSkillsKey)
}

/** The home page's entry to the Skills page. */
export function SkillsCard() {
  const skills = useQuery(['skills.list'], () => dataModel.skills())
  const available = skills.data?.filter((s) => s.enabled).length
  const disabled = skills.data?.filter((s) => !s.enabled).length
  return (
    <section className="card min-w-0" data-testid="skills-card">
      <a href="#/skills" className="flex min-h-14 items-center gap-3 px-4 py-3">
        <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-soft text-accent">
          <BookOpen size={18} />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block text-base font-semibold">{t('skills')}</span>
          <span className="block truncate text-xs text-mute" data-testid="skills-card-summary">
            {skills.data
              ? t('skSummary', { available: available ?? 0, disabled: disabled ?? 0 })
              : skills.error ? skills.error.message : t('loading')}
          </span>
        </span>
        <ChevronRight size={18} className="shrink-0 text-mute" />
      </a>
    </section>
  )
}

function CommandsDetail({ status }: { status: SkillCommandsStatus }) {
  const missing = Object.entries(status.commands).filter(([, c]) => !c.present).map(([name]) => name)
  const fromSystem = Object.entries(status.commands).filter(([, c]) => c.present && c.provided_by === 'system').map(([name]) => name)
  return (
    <>
      {missing.length > 0 && <div data-testid="skill-missing">{t('skMissingCommands', { commands: missing.join(', ') })}</div>}
      {status.recommended_packages?.map((p) => (
        <div key={p.id} data-testid="skill-recommended">
          {t('skRecommendedPackage', { pkg: `${p.id} ${p.version}`.trim(), commands: p.provides.join(', ') })}
          {p.requires_chain?.length ? ` · ${t('skRequiresChain', { chain: p.requires_chain.join(' → ') })}` : ''}
        </div>
      ))}
      {status.no_package?.length ? <div>{t('skNoPackage', { commands: status.no_package.join(', ') })}</div> : null}
      {status.state === 'ready_system' && (
        <div className="text-mute">{t('skReadySystemNote', { commands: fromSystem.join(', ') || '–' })}</div>
      )}
    </>
  )
}

/** The three levels of §3.6.2: not available / some steps lack commands / commands complete. */
function Availability({ skill, status, loading }: { skill: SkillMeta; status?: SkillRuntimeStatus; loading: boolean }) {
  const requires = status?.requires ?? skill.requires
  const declared = status ? Object.keys(status.commands.commands).length : 0
  return (
    <div className="min-w-[14rem] text-xs" data-testid={`skill-availability-${skill.name}`}>
      {!status ? (
        <span className="text-mute">{loading ? t('skChecking') : '–'}</span>
      ) : status.unmet ? (
        <>
          <Badge value={t('skUnavailable')} tone="bad" />
          <div className="mt-0.5 text-bad">{status.unmet}</div>
        </>
      ) : status.commands.state === 'partial' ? (
        <>
          <Badge value={t('skPartial')} tone="waiting" />
          <div className="mt-0.5"><CommandsDetail status={status.commands} /></div>
        </>
      ) : (
        <>
          <Badge value={t('skReady')} tone="ok" />
          <div className="mt-0.5">
            {declared === 0 ? <span className="text-mute">{t('skNoCommandsDeclared')}</span> : <CommandsDetail status={status.commands} />}
          </div>
        </>
      )}
      {requires && <div className="mt-0.5 text-mute">{t('skRequires')}: {requires}</div>}
    </div>
  )
}

function SkillRow({ skill, status, statusLoading, busy, run, ask }: {
  skill: SkillMeta
  status?: SkillRuntimeStatus
  statusLoading: boolean
  busy: boolean
  run: (action: () => Promise<unknown>) => void
  ask: (request: ConfirmRequest) => void
}) {
  const removable = skill.origin === 'installed' || skill.origin === 'imported'
  const title = `${t(KIND_LABEL[skill.kind])} · ${skill.name}`
  return (
    <tr data-testid={`skill-${skill.kind}-${skill.name}`} className={clsx(!skill.enabled && 'opacity-60')}>
      <td className="min-w-[14rem]">
        <Mono className="font-semibold">{skill.name}</Mono>
        {!skill.enabled && <span className="ml-1.5"><Badge value={t('skDisabled')} tone="finished" /></span>}
        <div className="text-xs text-mute">{skill.description || '–'}</div>
        {skill.disabled && (
          <div className="text-xs text-warn" data-testid="skill-disabled-reason">
            {t('skDisabledBy', { by: skill.disabled.by || '?', time: ago(skill.disabled.at_ms) })}
            {skill.disabled.reason ? `: ${skill.disabled.reason}` : ''}
          </div>
        )}
      </td>
      <td className="whitespace-nowrap"><Mono>{skill.scope ?? (skill.kind === 'work' ? 'work' : '–')}</Mono></td>
      <td className="whitespace-nowrap text-xs">
        {t(ORIGIN_LABEL[skill.origin] ?? 'skOriginLocal')}
        {skill.install && <div className="text-mute">{`${skill.install.package} ${skill.install.version}`.trim()}</div>}
      </td>
      <td className="whitespace-nowrap text-xs tabular-nums">
        {fmtBytes(skill.size)}
        {skill.references?.length ? <div className="text-mute" title={skill.references.join('\n')}>{t('skReferences', { n: skill.references.length })}</div> : null}
      </td>
      <td className="whitespace-nowrap text-xs">
        {skill.usage?.frozen ? (
          <>
            {t('skUsageFrozen', { n: skill.usage.frozen })}
            <div className="text-mute" title={new Date(skill.usage.last_ms).toLocaleString()}>{ago(skill.usage.last_ms)}</div>
          </>
        ) : (
          <span className="text-mute">{t('skNeverUsed')}</span>
        )}
      </td>
      <td>
        {skill.kind === 'work'
          ? <Availability skill={skill} status={status} loading={statusLoading} />
          : <span className="text-xs text-mute">{skill.requires ? `${t('skRequires')}: ${skill.requires}` : '–'}</span>}
      </td>
      <td>
        <div className="flex flex-wrap gap-1">
          {skill.enabled ? (
            <button className="btn" disabled={busy} onClick={() => ask({
              title: t('skDisableTitle', { name: title }),
              detail: t('skNewSessionsOnly'),
              inputLabel: t('skReasonOptional'),
              confirmLabel: t('skDisable'),
              run: (reason) => dataModel.setSkillEnabled(skill.kind, skill.name, false, reason || undefined),
            })}>{t('skDisable')}</button>
          ) : (
            <button className="btn" disabled={busy} onClick={() => run(() => dataModel.setSkillEnabled(skill.kind, skill.name, true))}>{t('skEnable')}</button>
          )}
          {removable && (
            <button className="btn border-bad text-bad" disabled={busy} onClick={() => ask({
              title: t('skRemoveTitle', { name: title }),
              detail: t('skRemoveHint'),
              confirmLabel: t('skRemove'),
              danger: true,
              run: () => dataModel.removeSkill(skill.kind, skill.name),
            })}>{t('skRemove')}</button>
          )}
        </div>
      </td>
    </tr>
  )
}

function UiSetPanel({ busy, run }: { busy: boolean; run: (action: () => Promise<unknown>) => void }) {
  const ui = useQuery(['skills.ui_get'], () => dataModel.uiSkills())
  const [adding, setAdding] = useState('')
  const names = ui.data?.names ?? []
  const available = ui.data?.available ?? []
  const pick = available.includes(adding) ? adding : available[0] ?? ''
  return (
    <Panel
      title={t('skUiSet')}
      testId="skills-ui-set"
      extra={ui.data && (
        <span className="text-xs text-mute" data-testid="skills-ui-source">
          {t(ui.data.source === 'state' ? 'skUiSourceState' : 'skUiSourceTemplate')} · {fmtBytes(ui.data.total_bytes)}
        </span>
      )}
    >
      <p className="mb-2 text-xs text-mute">{t('skUiHint')}</p>
      <ErrorBox error={ui.error} />
      <div className="mb-2 flex flex-wrap gap-1.5">
        {names.map((name) => (
          <span key={name} className="inline-flex items-center gap-1 rounded border border-line px-1.5 py-0.5" data-testid={`ui-skill-${name}`}>
            <Mono>{name}</Mono>
            <button className="text-xs text-bad hover:underline" disabled={busy} aria-label={`${t('skUiRemove')} ${name}`}
              onClick={() => run(() => dataModel.setUiSkills(names.filter((n) => n !== name)))}>
              {t('skUiRemove')}
            </button>
          </span>
        ))}
        {ui.data && names.length === 0 && <Empty>{t('skUiEmpty')}</Empty>}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <select className="input" aria-label={t('skUiAdd')} value={pick} disabled={!available.length} onChange={(e) => setAdding(e.target.value)}>
          {available.length ? available.map((name) => <option key={name} value={name}>{name}</option>) : <option value="">{t('skUiNothingToAdd')}</option>}
        </select>
        <button className="btn" disabled={busy || !pick} onClick={() => run(() => dataModel.setUiSkills([...names, pick]))}>{t('skUiAdd')}</button>
        <button className="btn" disabled={busy || ui.data?.source !== 'state'} onClick={() => run(() => dataModel.setUiSkills(null))}>{t('skUiReset')}</button>
      </div>
    </Panel>
  )
}

interface PlanRequest {
  source: SkillSource
  /** Tells one source from another in the query cache. */
  key: string
  title: string
  ignored?: string[]
}

/** §3.6.1 B: what the package holds, what it replaces and the risks; then the owner says yes. */
function PlanDialog({ request, onClose, onDone }: { request: PlanRequest; onClose: () => void; onDone: (plan: InstallPlan) => void }) {
  const plan = useQuery(['skills.install_plan', request.key], () => dataModel.skillInstallPlan(request.source), false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const data = plan.data
  const stale = plan.isValidating || plan.isLoading
  const replaced = data?.skills.filter((s) => s.replaces) ?? []
  const notInstallable = data?.risk.findings.some((f) => f.level === 'error') ?? false
  const partial = data?.command_status?.filter((c) => c.status.state === 'partial') ?? []

  const install = async () => {
    if (!data) return
    setBusy(true)
    setError(null)
    try {
      await dataModel.installSkills(request.source, data.plan_digest, replaced.length > 0)
      onDone(data)
      onClose()
    } catch (e) {
      setError(e)
      setBusy(false)
    }
  }

  return (
    <div className="fixed inset-0 z-10 flex items-center justify-center bg-black/40 p-4">
      <div role="dialog" aria-modal="true" aria-label={request.title} data-testid="skill-plan-dialog"
        className="max-h-[92dvh] w-full max-w-2xl overflow-y-auto rounded border border-line bg-panel p-4">
        <h3 className="mb-2 text-sm font-semibold">{request.title}</h3>
        <ErrorBox error={plan.error} />
        {!data && !plan.error && <Empty>{t('loading')}</Empty>}
        {data && (
          <div className={clsx('text-xs', stale && 'opacity-60')}>
            <div className="mb-2 text-mute">
              <Mono>{`${data.package} ${data.version}`.trim()}</Mono> · <Mono>{data.digest}</Mono>
            </div>
            <h4 className="mb-1 font-semibold text-mute">{t('skPlanSkills')}</h4>
            <ul className="mb-2" data-testid="plan-skills">
              {data.skills.map((s) => (
                <li key={`${s.kind}:${s.name}`} className="mb-1">
                  <Mono className="font-semibold">{s.kind}:{s.name}</Mono>
                  {s.scope && <Mono className="ml-1 text-mute">scope {s.scope}</Mono>}
                  {s.replaces && <span className="ml-1.5 text-warn" data-testid="plan-replaces">{t('skReplaces', { origin: s.replaces })}</span>}
                  <div className="text-mute">{s.description}</div>
                  {s.requires && <div className="text-mute">{t('skRequires')}: {s.requires}</div>}
                </li>
              ))}
            </ul>
            {partial.length > 0 && (
              <>
                <h4 className="mb-1 font-semibold text-mute">
                  {t('skPartial')}{data.checked_runtime_id ? ` · ${data.checked_runtime_id}` : ''}
                </h4>
                <ul className="mb-2">
                  {partial.map((c) => (
                    <li key={c.name} className="mb-1"><Mono>{c.name}</Mono><CommandsDetail status={c.status} /></li>
                  ))}
                </ul>
              </>
            )}
            <h4 className="mb-1 font-semibold text-mute">
              {t('skRisk')}: <span data-testid="plan-risk"><Badge value={t(RISK_LABEL[data.risk.level] ?? 'skRiskHigh')} tone={FINDING_TONE[data.risk.level]} /></span>
            </h4>
            {data.risk.findings.length ? (
              <ul className="mb-2" data-testid="plan-findings">
                {data.risk.findings.map((f, i) => (
                  <li key={`${f.code}-${f.where}-${i}`} className="mb-1">
                    <Badge value={f.level} tone={FINDING_TONE[f.level]} /> <Mono>{f.code}</Mono> <Mono className="text-mute">{f.where}</Mono>
                    <div>{f.note}</div>
                  </li>
                ))}
              </ul>
            ) : (
              <Empty>{t('skNoFindings')}</Empty>
            )}
            <p className="mb-2 text-mute" data-testid="plan-text-only">{t('skInstallTextOnly')}</p>
            {request.ignored?.length ? <p className="mb-2 text-mute">{t('skIgnoredFiles', { n: request.ignored.length })}</p> : null}
          </div>
        )}
        <ErrorBox error={error} />
        <div className="flex justify-end gap-2">
          <button className="btn" onClick={onClose} disabled={busy}>{t('cancel')}</button>
          {data && !notInstallable && (
            <button className={clsx('btn', data.risk.level === 'high' ? 'border-bad text-bad' : 'border-accent text-accent')}
              onClick={() => void install()} disabled={busy || stale} data-testid="plan-confirm">
              {t(replaced.length ? 'skInstallReplace' : 'skInstall')}
            </button>
          )}
        </div>
      </div>
    </div>
  )
}

const PIKG_ACCEPT = [{ ext: ['.pikg', '.zip'] }, { mime: 'application/zip' }]

function InstallPanel({ plan }: { plan: (request: PlanRequest) => void }) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const pick = async () => {
    setBusy(true)
    setError(null)
    try {
      const file = await pickFile(PIKG_ACCEPT)
      if (!file) return
      const read = await readSkillPikg(file)
      plan({
        source: { package: read.package },
        key: read.package.pikg_digest ?? file.name,
        title: t('skInstallTitle', { name: `${read.package.package} ${read.package.version}`.trim() }),
        ignored: read.ignored,
      })
    } catch (e) {
      setError(e)
    } finally {
      setBusy(false)
    }
  }
  return (
    <Panel title={t('skInstallFromFile')} testId="skills-install">
      <p className="mb-2 text-xs text-mute">{t('skInstallHint')}</p>
      <ErrorBox error={error} />
      <button className="btn" disabled={busy} data-testid="skills-pikg-choose" onClick={() => void pick()}>{t(busy ? 'loading' : 'skChoosePikg')}</button>
    </Panel>
  )
}

function ImportPanel() {
  const refresh = useSkillsRefresh()
  const [text, setText] = useState('')
  const [name, setName] = useState('')
  const [scope, setScope] = useState('')
  const [source, setSource] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const [session, setSession] = useState<string | null>(null)
  const load = async () => {
    try {
      const picked = await pickFile()
      if (!picked) return
      setText(await picked.text())
      setSource(picked.name)
      setError(null)
    } catch (e) {
      setError(e)
    }
  }
  const submit = async () => {
    setBusy(true)
    setError(null)
    setSession(null)
    try {
      const result = await dataModel.importSkill({
        text,
        ...(name.trim() ? { name: name.trim() } : {}),
        ...(scope ? { scope } : {}),
        ...(source ? { source } : {}),
      })
      setSession(result.session_id)
      setText('')
      setName('')
      setSource('')
      refresh()
    } catch (e) {
      setError(e)
    } finally {
      setBusy(false)
    }
  }
  return (
    <Panel title={t('skImport')} testId="skills-import">
      <p className="mb-2 text-xs text-mute">{t('skImportHint')}</p>
      <textarea className="input mb-2 block w-full font-mono" rows={6} value={text} aria-label={t('skImportText')} placeholder={t('skImportText')}
        onChange={(e) => { setText(e.target.value); setSource('') }} />
      <div className="mb-2 flex flex-wrap items-center gap-2 text-xs text-mute">
        <button className="btn" data-testid="skills-import-file" onClick={() => void load()}>{t('skImportFile')}</button>
        {source && <Mono>{source}</Mono>}
        <label>
          {t('skImportName')} <input className="input ml-1" value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label>
          {t('skScope')}
          <select className="input ml-1" value={scope} onChange={(e) => setScope(e.target.value)}>
            <option value="">{t('skScopeAuto')}</option>
            <option value="work">work</option>
            <option value="ui">ui</option>
            <option value="plan">plan</option>
          </select>
        </label>
        <button className="btn border-accent text-accent" disabled={busy || !text.trim()} onClick={() => void submit()}>{t('skImportStart')}</button>
      </div>
      <ErrorBox error={error} />
      {session && (
        <p className="text-xs text-ok" data-testid="skills-import-started">
          {t('skImportStarted')} <a className="font-mono text-accent hover:underline" href={sessionHref(session)}>{session}</a>
        </p>
      )}
    </Panel>
  )
}

const draftField = (draft: unknown, key: string): unknown =>
  draft && typeof draft === 'object' ? (draft as Record<string, unknown>)[key] : undefined

function DraftFile({ slug, file }: { slug: string; file: string }) {
  const text = useQuery(['skills.draft_file', slug, file], () => dataModel.skillDraftFile(slug, file), false)
  return (
    <>
      <ErrorBox error={text.error} />
      {text.data ? <Pre value={text.data.text} /> : !text.error && <Empty>{t('loading')}</Empty>}
    </>
  )
}

function DraftView({ draft }: { draft: SkillDraft }) {
  const [file, setFile] = useState<string | null>(null)
  const risk = draftField(draft.draft, 'risk')
  const verified = draftField(draft.draft, 'verified')
  return (
    <div className="mt-2 rounded border border-line p-2 text-xs" data-testid={`draft-view-${draft.slug}`}>
      {risk != null && (
        <div className="mb-2 rounded border border-warn px-2 py-1" data-testid="draft-risk">
          <div className="font-semibold text-warn">{t('skDraftRisk')}</div>
          <div className="whitespace-pre-wrap">{toText(risk)}</div>
        </div>
      )}
      {verified != null && (
        <div className="mb-2">
          <div className="font-semibold text-mute">{t('skDraftVerified')}</div>
          <Pre value={verified} />
        </div>
      )}
      <div className="mb-1 font-semibold text-mute">draft.json</div>
      {draft.draft != null ? <Pre value={draft.draft} /> : <Empty />}
      <div className="mb-1 mt-2 font-semibold text-mute">{t('skDraftFiles')}</div>
      <div className="mb-2 flex flex-wrap gap-1">
        {draft.files.map((f) => (
          <button key={f} className={clsx('btn font-mono', file === f && 'border-accent text-accent')} onClick={() => setFile(file === f ? null : f)}>{f}</button>
        ))}
        {draft.files.length === 0 && <Empty />}
      </div>
      {file && <DraftFile key={file} slug={draft.slug} file={file} />}
    </div>
  )
}

function DraftsPanel({ plan, ask }: { plan: (request: PlanRequest) => void; ask: (request: ConfirmRequest) => void }) {
  const drafts = useQuery(['skills.drafts'], () => dataModel.skillDrafts())
  const [open, setOpen] = useState<string | null>(null)
  const sorted = [...(drafts.data ?? [])].sort((a, b) => b.updated_at_ms - a.updated_at_ms)
  return (
    <Panel title={`${t('skDrafts')} (${sorted.length})`} testId="skills-drafts">
      <p className="mb-2 text-xs text-mute">{t('skDraftsHint')}</p>
      <ErrorBox error={drafts.error} />
      {sorted.length === 0 && !drafts.error && <Empty>{t(drafts.isLoading ? 'loading' : 'skNoDrafts')}</Empty>}
      <ul className="divide-y divide-line">
        {sorted.map((draft) => (
          <li key={draft.slug} className="py-2" data-testid={`draft-${draft.slug}`}>
            <div className="flex flex-wrap items-center gap-2">
              <Mono className="font-semibold">{draft.slug}</Mono>
              <Time ms={draft.updated_at_ms} />
              <span className="ml-auto flex gap-1">
                <button className="btn" onClick={() => setOpen(open === draft.slug ? null : draft.slug)}>{t(open === draft.slug ? 'skDraftHide' : 'skDraftView')}</button>
                <button className="btn border-accent text-accent" disabled={Boolean(draft.error)} onClick={() => plan({
                  source: { draft: draft.slug },
                  key: `draft:${draft.slug}:${draft.updated_at_ms}`,
                  title: t('skInstallTitle', { name: draft.slug }),
                })}>{t('skInstall')}</button>
                <button className="btn border-bad text-bad" onClick={() => ask({
                  title: t('skDiscardTitle', { name: draft.slug }),
                  detail: t('skDiscardHint'),
                  confirmLabel: t('skDiscard'),
                  danger: true,
                  run: () => dataModel.discardSkillDraft(draft.slug),
                })}>{t('skDiscard')}</button>
              </span>
            </div>
            {draft.skills.map((s) => (
              <div key={`${s.kind}:${s.name}`} className="text-xs">
                <Mono>{s.kind}:{s.name}</Mono>{s.scope && <Mono className="ml-1 text-mute">scope {s.scope}</Mono>}
                <span className="ml-1.5 text-mute">{s.description}</span>
              </div>
            ))}
            {draft.error && <div className="text-xs text-bad" data-testid="draft-error">{draft.error}</div>}
            {open === draft.slug && <DraftView draft={draft} />}
          </li>
        ))}
      </ul>
    </Panel>
  )
}

function AuditPanel() {
  const state = useQuery(['skills.state'], () => dataModel.skillsState())
  const audit = [...(state.data?.audit ?? [])].reverse()
  const none = state.data?.usage?.none
  return (
    <Panel title={t('skAudit')} testId="skills-audit">
      <ErrorBox error={state.error} />
      {none?.frozen ? (
        <p className="mb-2 text-xs text-mute" data-testid="skills-usage-none">
          {t('skUsageNone', { n: none.frozen })}
          {none.recent_sessions?.length ? <> · {none.recent_sessions.slice(-3).reverse().map((sid) => <span key={sid} className="mr-1.5"><Sid sid={sid} /></span>)}</> : null}
        </p>
      ) : null}
      <details>
        <summary className="cursor-pointer text-xs text-accent" data-testid="skills-audit-toggle">{t('skAuditShow', { n: audit.length })}</summary>
        {audit.length ? (
          <table className="tbl mt-2">
            <thead>
              <tr><th>{t('skAuditTime')}</th><th>{t('skAuditBy')}</th><th>{t('skAuditAction')}</th><th>{t('skAuditSkill')}</th><th>{t('skAuditNote')}</th></tr>
            </thead>
            <tbody>
              {audit.map((a, i) => (
                <tr key={`${a.at_ms}-${i}`} data-testid="audit-row">
                  <td><Time ms={a.at_ms} /></td>
                  <td><Mono>{a.by}</Mono></td>
                  <td><Mono>{a.action}</Mono></td>
                  <td><Mono>{a.skill ?? ''}</Mono></td>
                  <td className="text-xs">{a.note ?? ''}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <Empty>{t('skNoAudit')}</Empty>
        )}
      </details>
    </Panel>
  )
}

const runtimeLabel = (runtime: KnownRuntime) =>
  `${runtime.name} · ${runtime.runtime_id}${runtime.caps.os ? ` · ${runtime.caps.os}` : ''}${runtime.lifecycle === 'retired' ? ` · ${t('runtimeRetired')}` : ''}`

export default function SkillsPage() {
  const refresh = useSkillsRefresh()
  const skills = useQuery(['skills.list'], () => dataModel.skills())
  const runtimes = useQuery(['runtimes.list'], () => dataModel.runtimes())
  const [chosen, setChosen] = useState<string>()
  // The private runtime is the default; its record may carry another id than the alias.
  const runtimeId = chosen ?? runtimes.data?.find((r) => r.connector.type === 'native')?.runtime_id ?? PRIVATE
  // The service checks the commands on the runtime now: seconds, so not polled and never in the way of the list.
  const status = useQuery(['skills.status', runtimeId], () => dataModel.skillsStatus(runtimeId), false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const [notice, setNotice] = useState<ReactNode>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const [plan, setPlan] = useState<PlanRequest | null>(null)

  const run = (action: () => Promise<unknown>) => {
    setBusy(true)
    setError(null)
    setNotice(null)
    action()
      .catch(setError)
      .finally(() => {
        setBusy(false)
        refresh()
      })
  }
  // A confirmed write reloads the page's data once it is through.
  const ask = (request: ConfirmRequest) =>
    setConfirm({ ...request, run: async (input) => { await request.run(input); refresh() } })

  const all = skills.data ?? []
  const selectable = (runtimes.data ?? []).filter((r) => r.lifecycle === 'active' || r.runtime_id === runtimeId)
  // A status of another runtime (kept while the new one loads) says nothing about this one.
  const current = status.data && (status.data.runtime_id === runtimeId || !status.isValidating) ? status.data : undefined
  const byName = new Map((current?.skills ?? []).map((s) => [s.name, s]))
  const statusRuntime = current?.runtime_id ?? runtimeId

  return (
    <div data-testid="skills-page">
      <div className="mb-2 flex flex-wrap items-center gap-x-4 gap-y-1">
        <h1 className="text-base font-semibold">{t('skills')}</h1>
        {skills.data && (
          <span className="text-xs text-mute" data-testid="skills-summary">
            {t('skSummary', { available: all.filter((s) => s.enabled).length, disabled: all.filter((s) => !s.enabled).length })}
          </span>
        )}
        <label className="ml-auto text-xs text-mute">
          {t('skRuntime')}
          <select className="input ml-1" value={runtimeId} onChange={(e) => setChosen(e.target.value)} data-testid="skills-runtime-select">
            {!selectable.some((r) => r.runtime_id === runtimeId) && <option value={runtimeId}>{runtimeId}</option>}
            {selectable.map((r) => <option key={r.runtime_id} value={r.runtime_id}>{runtimeLabel(r)}</option>)}
          </select>
        </label>
        <button className="btn" disabled={status.isValidating} onClick={() => void status.mutate()} data-testid="skills-status-refresh">
          {t(status.isValidating ? 'skChecking' : 'skCheckRuntime')}
        </button>
      </div>
      <p className="mb-2 text-xs text-mute">{t('skAvailabilityHint')}</p>
      <ErrorBox error={skills.error ?? runtimes.error ?? error} />
      {status.error && <ErrorBox error={status.error} />}
      {notice && <div className="mb-3 rounded border border-ok px-3 py-2 text-xs text-ok" data-testid="skills-notice">{notice}</div>}

      {KINDS.map((kind) => {
        const rows = all.filter((s) => s.kind === kind).sort((a, b) => Number(hasUi(b.scope)) - Number(hasUi(a.scope)) || a.name.localeCompare(b.name))
        if (kind === 'goal' && rows.length === 0) return null
        return (
          <Panel key={kind} title={`${t(KIND_LABEL[kind])} (${rows.length})`} testId={`skills-${kind}`}>
            {rows.length ? (
              <table className="tbl">
                <thead>
                  <tr>
                    <th>{t('skName')}</th>
                    <th>{t('skScope')}</th>
                    <th>{t('skOrigin')}</th>
                    <th>{t('skSize')}</th>
                    <th>{t('skUsage')}</th>
                    <th>{kind === 'work' ? t('skAvailabilityOn', { runtime: statusRuntime }) : t('skRequires')}</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {rows.map((skill) => (
                    <SkillRow key={skill.name} skill={skill} status={byName.get(skill.name)} statusLoading={status.isValidating || status.isLoading}
                      busy={busy} run={run} ask={ask} />
                  ))}
                </tbody>
              </table>
            ) : (
              <Empty>{t(skills.isLoading ? 'loading' : 'skNone')}</Empty>
            )}
          </Panel>
        )
      })}

      <UiSetPanel busy={busy} run={run} />
      <InstallPanel plan={setPlan} />
      <DraftsPanel plan={setPlan} ask={ask} />
      <ImportPanel />
      <AuditPanel />

      {confirm && <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />}
      {plan && (
        <PlanDialog
          key={plan.key}
          request={plan}
          onClose={() => setPlan(null)}
          onDone={(done) => {
            setNotice(t('skInstalled', { skills: done.skills.map((s) => s.name).join(', ') }))
            refresh()
          }}
        />
      )}
    </div>
  )
}
