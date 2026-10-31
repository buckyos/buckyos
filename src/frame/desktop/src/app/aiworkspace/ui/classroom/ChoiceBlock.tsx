/* eslint-disable react-refresh/only-export-components -- a Block definition with its renderer and inspector */
/* The choice question (第四期规划 §5.2, §5.4; the classroom extension's control): a question with options the author
 * writes in the courseware. In a session the presenter asks it — open to everyone, to one participant picked at
 * random, or to the first N who claim it — sees the answers come in, closes it and publishes the result; a participant
 * answers when the question is theirs. The rules are this control's (default classroom extension, Q08/Q09):
 *   - a question is `{ source workspace, block, content revision }`; asking it again is a new task;
 *   - a pick draws from the online, verified participants (∩ the roster when one is bound) who have not been picked or
 *     answered in this Block, with a seed that goes into the task; the replay reads the pick, it never draws again;
 *   - a claim slot is the kernel's (`session.claim`): never more than N, a disconnection gives nothing back;
 *   - a revoked participant's answer stays, marked invalid; late answers are refused (and recorded).
 * Outside a session (the courseware) it shows the question and binds the feeds it will read. */

import { useEffect, useMemo, useState } from 'react'
import { describeError } from '../../api/session'
import type { Json, KeyedContent, QueryRow, TableSourceContent } from '../../api/types'
import { randomId } from '../../api/ids'
import { useLoad, useStore } from '../../state/hooks'
import { useSession } from '../../session/context'
import { useAiwsText } from '../../useAiwsText'
import type { BlockDefinition, RenderContext } from '../blocks/registry'
import { cellOp } from '../blocks/ops'
import { aiwsT } from '../../i18n'
import { boundId, ensureBindings, json, rosterColumns, seeded, text, useRows } from './feeds'
import { TASK_MODE_KEY, TASK_STATE_KEY } from '../../session/labels'

export const CHOICE_VIEW = 'classroom.choice'

const FEEDS = [
  { name: 'tasks', feed: 'tasks' },
  { name: 'my_responses', feed: 'my_responses' },
  { name: 'responses', feed: 'responses' },
  { name: 'results', feed: 'results' },
  { name: 'participants', feed: 'participants' },
] as const

interface ChoiceConfig { question: string; options: string[]; multiple: boolean; slots: number }

function configOf(context: RenderContext): ChoiceConfig {
  const c = (context.payload.config ?? {}) as Record<string, Json>
  const options = Array.isArray(c.options) ? c.options.filter((o): o is string => typeof o === 'string') : []
  return { question: typeof c.question === 'string' ? c.question : '', options: options.length ? options : ['A', 'B', 'C', 'D'], multiple: c.multiple === true, slots: typeof c.slots === 'number' ? c.slots : 1 }
}

function letter(i: number): string { return String.fromCharCode(65 + i) }

function answerOf(row: QueryRow | undefined): number[] {
  const v = json(row, 'content')
  if (v && typeof v === 'object' && !Array.isArray(v) && Array.isArray((v as Record<string, Json>).choice)) return ((v as Record<string, Json>).choice as Json[]).filter((x): x is number => typeof x === 'number')
  return []
}

function ChoiceStatic(context: RenderContext) {
  const store = useStore()
  const session = useSession()
  const { t } = useAiwsText()
  const config = configOf(context)
  // the courseware binds the feeds this control reads (the author's write; nothing in a session or a replay)
  useEffect(() => { if (!session && context.mode === 'edit') ensureBindings(context, store, [...FEEDS]) }, [session, context, store])
  const tasks = useRows(store, boundId(context, store, session, 'tasks'))
  const mine = useRows(store, session && session.role === 'audience' ? boundId(context, store, session, 'my_responses') : undefined)
  const responses = useRows(store, session && session.role !== 'audience' && session.role !== 'display' ? boundId(context, store, session, 'responses') : undefined)
  const results = useRows(store, boundId(context, store, session, 'results'))
  const blockTasks = (tasks ?? []).filter((r) => text(r, 'block_id') === context.cell.entity_id).sort((a, b) => text(a, 'created_at').localeCompare(text(b, 'created_at')))
  const task = blockTasks.at(-1)
  const taskId = task ? text(task, 'task_id') : null
  const result = (results ?? []).filter((r) => text(r, 'task_id') === taskId).at(-1)
  const presenting = session && !session.replay && (session.role === 'presenter' || session.role === 'prompter')
  const reviewer = session && (session.role === 'presenter' || session.role === 'owner' || session.role === 'prompter')
  const answers = (responses ?? []).filter((r) => text(r, 'task_id') === taskId && text(r, 'state') === 'submitted')
  const counts = config.options.map((_, i) => answers.filter((r) => answerOf(r).includes(i)).length)
  return (
    <div className="aiws-classroom aiws-classroom-choice" data-testid={`aiws-choice-${context.cell.entity_id}`} data-task={taskId ?? undefined} data-state={task ? text(task, 'state') : 'none'}>
      <div className="aiws-classroom-question">{config.question || <span className="aiws-muted">{t('aiworkspace.session.choice.noQuestion')}</span>}</div>
      {session && session.role === 'audience' && !session.replay
        ? <Answer context={context} config={config} task={task} mine={mine} />
        : (
          <ol className="aiws-classroom-options">
            {config.options.map((o, i) => (
              <li key={i} className={result && answerOf(result).includes(i) ? 'is-correct' : ''}>
                <span className="aiws-classroom-letter">{letter(i)}</span><span className="aiws-grow">{o}</span>
                {reviewer && taskId && <span className="aiws-classroom-count" data-testid={`aiws-choice-count-${i}`}>{counts[i]}</span>}
              </li>
            ))}
          </ol>
        )}
      {result && <div className="aiws-classroom-result" data-testid="aiws-choice-result">{t('aiworkspace.session.choice.published', { answer: answerOf(result).map(letter).join(', ') || text(result, 'content') })}</div>}
      {task && <div className="aiws-muted aiws-classroom-meta" data-testid="aiws-choice-meta">{t(TASK_MODE_KEY[text(task, 'mode')] ?? TASK_MODE_KEY.open)} · {t(TASK_STATE_KEY[text(task, 'state')] ?? TASK_STATE_KEY.open)}{text(task, 'picked') ? ` · ${t('aiworkspace.session.choice.picked', { names: text(task, 'picked') })}` : ''} · {t('aiworkspace.session.choice.answered', { count: Number(task.values.answered ?? 0) })}</div>}
      {presenting && <Controls context={context} config={config} task={task} answers={answers} responses={responses ?? []} blockTasks={blockTasks} counts={counts} />}
      {!session && context.mode !== 'show' && <div className="aiws-muted aiws-classroom-meta">{t('aiworkspace.session.choice.designHint')}</div>}
    </div>
  )
}

/** The participant's side: claim, answer, the confirmation; nothing when the question is not theirs. */
function Answer({ context, config, task, mine }: { context: RenderContext; config: ChoiceConfig; task: QueryRow | undefined; mine: QueryRow[] | null }) {
  const session = useSession()!
  const { t } = useAiwsText()
  const [picked, setPicked] = useState<number[]>([])
  const [problem, setProblem] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const taskId = task ? text(task, 'task_id') : null
  const own = (mine ?? []).find((r) => text(r, 'task_id') === taskId)
  const state = own ? text(own, 'state') : null
  const open = task && text(task, 'state') === 'open'
  const mode = task ? text(task, 'mode') : 'open'
  const mayAnswer = open && session.mayInteract && (mode === 'open' || state === 'assigned' || state === 'claimed' || state === 'submitted')
  const call = async (method: string, params: Record<string, unknown>) => {
    setBusy(true)
    try { await session.channel.call(method, params); setProblem(null) } catch (e) { setProblem(describeError(e)) } finally { setBusy(false) }
  }
  const toggle = (i: number) => setPicked(config.multiple ? (picked.includes(i) ? picked.filter((x) => x !== i) : [...picked, i]) : [i])
  return (
    <div className="aiws-classroom-answer" data-testid="aiws-choice-answer" data-mine={state ?? 'none'}>
      {state === 'assigned' && open && <div className="aiws-classroom-called" data-testid="aiws-choice-called">{t('aiworkspace.session.choice.yourTurn')}</div>}
      <ol className="aiws-classroom-options">
        {config.options.map((o, i) => (
          <li key={i}>
            <button type="button" className={picked.includes(i) || answerOf(own).includes(i) ? 'is-on' : ''} disabled={!mayAnswer || busy} data-testid={`aiws-choice-option-${i}`}
              onPointerDown={(e) => e.stopPropagation()} onClick={() => toggle(i)}>
              <span className="aiws-classroom-letter">{letter(i)}</span>{o}
            </button>
          </li>
        ))}
      </ol>
      {open && mode === 'claim' && !own && session.mayInteract && <button type="button" className="is-primary" data-testid="aiws-choice-claim" disabled={busy} onPointerDown={(e) => e.stopPropagation()} onClick={() => { void call('session.claim', { task_id: taskId }) }}>{t('aiworkspace.session.choice.claim')}</button>}
      {mayAnswer && <button type="button" className="is-primary" data-testid="aiws-choice-submit" disabled={busy || picked.length === 0} onPointerDown={(e) => e.stopPropagation()}
        onClick={() => { void call('session.respond', { task_id: taskId, content: { choice: picked, block_id: context.cell.entity_id }, command_id: randomId('ans') }) }}>{state === 'submitted' ? t('aiworkspace.session.choice.resubmit') : t('aiworkspace.session.choice.submit')}</button>}
      {state === 'submitted' && <div className="aiws-muted" data-testid="aiws-choice-submitted">{t('aiworkspace.session.choice.submitted', { answer: answerOf(own).map(letter).join(', ') })}</div>}
      {state === 'invalid' && <div className="aiws-warning">{t('aiworkspace.session.choice.revoked')}</div>}
      {!session.mayInteract && <div className="aiws-muted">{t('aiworkspace.session.audience.watchOnly')}</div>}
      {problem && <div className="aiws-warning" role="status" data-testid="aiws-choice-problem">{problem}</div>}
    </div>
  )
}

/** The presenter's side (§5.4): ask, pick, claim, close, publish. */
function Controls({ context, config, task, answers, responses, blockTasks, counts }: { context: RenderContext; config: ChoiceConfig; task: QueryRow | undefined; answers: QueryRow[]; responses: QueryRow[]; blockTasks: QueryRow[]; counts: number[] }) {
  const session = useSession()!
  const store = useStore()
  const { t } = useAiwsText()
  const [problem, setProblem] = useState<string | null>(null)
  const participants = useRows(store, boundId(context, store, session, 'participants'))
  const rosterId = typeof context.payload.config?.roster === 'string' ? context.payload.config.roster as string : undefined
  const roster = useRows(store, rosterId)
  const loadFields = useMemo(() => async () => (rosterId ? (await store.session.read<KeyedContent<never> & TableSourceContent>(rosterId).catch(() => null)) : null), [store, rosterId])
  const rosterTable = useLoad(loadFields, rosterId ?? '').data
  const taskId = task ? text(task, 'task_id') : null
  const open = task && text(task, 'state') === 'open'
  const question = { workspace_id: session.info.source_workspace_id, entity_id: context.cell.entity_id, content_rev: context.cell.content_rev }
  const call = async (method: string, params: Record<string, unknown>) => {
    try { await session.channel.call(method, params); setProblem(null) } catch (e) { setProblem(describeError(e)) }
  }
  const create = (mode: 'open' | 'assigned' | 'claim', extra: Record<string, unknown> = {}) => call('session.task', { op: 'create', command_id: randomId('task'), task: { title: config.question.slice(0, 200) || t('aiworkspace.session.choice.untitled'), mode, block_id: context.cell.entity_id, question, ...(mode === 'claim' ? { slots: Math.max(1, config.slots) } : {}), ...extra }, ...(extra.principals ? { principals: extra.principals } : {}) })
  // the pool: online verified participants, in the roster when one is bound, not yet picked or answered in this Block
  const pick = () => {
    const touched = new Set(responses.filter((r) => blockTasks.some((bt) => text(bt, 'task_id') === text(r, 'task_id'))).map((r) => text(r, 'participant')))
    const fields: Record<string, string> = {}
    for (const f of (rosterTable as unknown as { content?: TableSourceContent } | null)?.content?.fields ?? []) fields[f.field_id] = f.name
    const cols = rosterColumns(roster, fields)
    const listed = roster && cols.principal ? new Set(roster.map((r) => text(r, cols.principal!)).filter(Boolean)) : null
    const pool = (participants ?? []).filter((p) => p.values.online === true && text(p, 'role') === 'audience' && text(p, 'level') !== 'guest')
      .map((p) => text(p, 'principal')).filter((p) => p && !touched.has(p) && (!listed || listed.has(p))).sort()
    if (pool.length === 0) { setProblem(t('aiworkspace.session.choice.poolEmpty')); return }
    const seed = crypto.getRandomValues(new Uint32Array(1))[0]
    const chosen = pool[Math.floor(seeded(seed)() * pool.length)]
    void create('assigned', { principals: [chosen], pick: { seed, pool, picked: [chosen], roster: rosterId ?? null } })
  }
  const assigned = responses.filter((r) => text(r, 'task_id') === taskId && text(r, 'state') !== 'invalid' && text(r, 'state') !== 'submitted')
  const publish = () => {
    const best = counts.length ? counts.indexOf(Math.max(...counts)) : 0
    const raw = window.prompt(t('aiworkspace.session.choice.publishPrompt'), letter(best))
    if (raw === null) return
    const choice = raw.toUpperCase().split(/[\s,，]+/).filter(Boolean).map((c) => c.charCodeAt(0) - 65).filter((i) => i >= 0 && i < config.options.length)
    void call('session.publish', { task_id: taskId, title: config.question.slice(0, 200), content: { choice, distribution: counts, answered: answers.length }, command_id: randomId('pub') })
  }
  return (
    <div className="aiws-classroom-controls" data-testid="aiws-choice-controls" onPointerDown={(e) => e.stopPropagation()}>
      {!open && <>
        <button type="button" data-testid="aiws-choice-ask" onClick={() => { void create('open') }}>{t('aiworkspace.session.choice.askAll')}</button>
        <button type="button" data-testid="aiws-choice-pick" onClick={pick}>{t('aiworkspace.session.choice.pick')}</button>
        <button type="button" data-testid="aiws-choice-race" onClick={() => { void create('claim') }}>{t('aiworkspace.session.choice.race', { count: Math.max(1, config.slots) })}</button>
      </>}
      {open && <>
        {assigned.map((r) => <button key={text(r, 'participant')} type="button" data-testid="aiws-choice-revoke" onClick={() => { void call('session.task', { op: 'revoke', task_id: taskId, principals: [text(r, 'participant')], command_id: randomId('task') }) }}>{t('aiworkspace.session.choice.revoke', { name: text(r, 'name') })}</button>)}
        <button type="button" data-testid="aiws-choice-close" onClick={() => { void call('session.task', { op: 'close', task_id: taskId, command_id: randomId('task') }) }}>{t('aiworkspace.session.choice.close')}</button>
      </>}
      {task && !open && answers.length > 0 && <button type="button" className="is-primary" data-testid="aiws-choice-publish" onClick={publish}>{t('aiworkspace.session.choice.publish')}</button>}
      {problem && <span className="aiws-warning" role="status" data-testid="aiws-choice-problem">{problem}</span>}
    </div>
  )
}

/** The author's side in the courseware: the question, the options, multiple answers, claim slots, the roster. */
function ChoiceInspector(context: RenderContext) {
  const store = useStore()
  const { t } = useAiwsText()
  const config = configOf(context)
  const [draft, setDraft] = useState(() => ({ question: config.question, options: config.options.join('\n'), multiple: config.multiple, slots: config.slots, roster: typeof context.payload.config?.roster === 'string' ? context.payload.config.roster as string : '' }))
  const tables = store.outline.all().filter((e) => e.type_id === 'buckyos.table-source' && !e.source && !e.deleted)
  const save = () => {
    const next: Record<string, Json> = { ...(context.payload.config ?? {}), question: draft.question, options: draft.options.split('\n').map((s) => s.trim()).filter(Boolean).slice(0, 8), multiple: draft.multiple, slots: Math.max(1, Math.min(100, draft.slots)) }
    if (draft.roster) next.roster = draft.roster
    else delete next.roster
    void store.submit({ editId: `config:${context.cell.entity_id}`, label: t('aiworkspace.session.choice.edit'), operations: [{ op: 'entity.set_keys', entity_id: context.cell.entity_id, keys: [{ key: 'config', value: next, expect: { rev: context.keyRevs.config ?? 0 } }] }] })
  }
  return (
    <div className="aiws-inspector-form" data-testid="aiws-choice-inspector">
      <label>{t('aiworkspace.session.choice.questionLabel')}<textarea value={draft.question} rows={3} onChange={(e) => setDraft({ ...draft, question: e.target.value })} /></label>
      <label>{t('aiworkspace.session.choice.optionsLabel')}<textarea value={draft.options} rows={5} onChange={(e) => setDraft({ ...draft, options: e.target.value })} /></label>
      <label className="aiws-check"><input type="checkbox" checked={draft.multiple} onChange={(e) => setDraft({ ...draft, multiple: e.target.checked })} />{t('aiworkspace.session.choice.multiple')}</label>
      <label>{t('aiworkspace.session.choice.slotsLabel')}<input type="number" min={1} max={100} value={draft.slots} onChange={(e) => setDraft({ ...draft, slots: Number(e.target.value) || 1 })} /></label>
      <label>{t('aiworkspace.session.choice.rosterLabel')}
        <select value={draft.roster} onChange={(e) => setDraft({ ...draft, roster: e.target.value })}>
          <option value="">{t('aiworkspace.session.choice.noRoster')}</option>
          {tables.map((e) => <option key={e.entity_id} value={e.entity_id}>{e.title ?? e.name ?? e.entity_id}</option>)}
        </select>
      </label>
      <button type="button" className="is-primary" onClick={save}>{t('aiworkspace.common.save')}</button>
    </div>
  )
}

export const choiceDefinition: BlockDefinition = {
  type: CHOICE_VIEW, version: 1, get title() { return aiwsT('aiworkspace.session.choice.title') }, accepts: [], allowNoSource: true, pureUi: true,
  defaultSize: { w: 420, h: 300 }, cost: { editor: false, html: false },
  Static: ChoiceStatic, Inspector: ChoiceInspector,
  catalog: { group: 'data', get description() { return aiwsT('aiworkspace.session.choice.description') }, needs: 'none' },
  create: (args) => [cellOp(args, CHOICE_VIEW, undefined, { config: { question: aiwsT('aiworkspace.session.choice.sampleQuestion'), options: ['A', 'B', 'C', 'D'], multiple: false, slots: 1, ...(args.config ?? {}) } })],
}
