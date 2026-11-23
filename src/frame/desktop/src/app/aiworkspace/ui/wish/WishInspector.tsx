/* A wish's settings in the properties panel (许愿格交互升级 §3.7): its inputs (bound by hand where the analysis could not
 * find one), refinements, long-lived knowledge, how results are written and, in this browser, where its runs execute.
 * Changing the inputs, refinements or knowledge first says what it makes out of date (§3.5). The Block's "显示连线" is a
 * config field of the wish Block (`config.show_links`). */

import { useState } from 'react'
import { X } from 'lucide-react'
import { SHOWS_ID, SYSTEM_IDS, type Json, type WishExecutionMode, type WishInput, type WishRefinement } from '../../api/types'
import { useOutlineVersion, useReadOnlyReason, useStore, useWorkspaceUi } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { useAiwsText } from '../../useAiwsText'
import type { RenderContext } from '../blocks/registry'
import { useInvalidation } from './confirm'
import { setWishKey, useWish } from './useWish'
import { EXECUTION_LABEL } from './WishService'
import './wish.css'

const PROBLEM_LABEL: Record<string, string> = {
  missing: 'aiworkspace.wish.inputProblem.missing', view_missing: 'aiworkspace.wish.inputProblem.viewMissing', unreadable: 'aiworkspace.wish.inputProblem.unreadable',
}

/** What of an input is read: all of it, through a saved table view, or a query (with how many columns, filtered). */
function scopeText(input: WishInput, title: (id: string) => string, t: (key: string, vars?: Record<string, string | number>) => string): string {
  const s = input.selector as { kind?: string; cell_id?: string; filter?: Json; fields?: string[] } | undefined
  if (!s || s.kind === 'entity') return t('aiworkspace.wish.scope.whole')
  if (s.kind === 'table_view') return t('aiworkspace.wish.scope.view', { view: title(s.cell_id ?? '') })
  if (s.kind === 'table_query') {
    if (s.fields) return t(s.filter ? 'aiworkspace.wish.scope.queryFieldsFiltered' : 'aiworkspace.wish.scope.queryFields', { count: s.fields.length })
    return t(s.filter ? 'aiworkspace.wish.scope.queryFiltered' : 'aiworkspace.wish.scope.query')
  }
  return s.kind ?? ''
}

/** The objects of the data tree a wish may read (no Blocks, wishes, definitions, system or show entities). */
function inputCandidates(store: WorkspaceStore, wishId: string, taken: readonly WishInput[]) {
  return store.outline.all().filter((e) => !e.deleted && e.entity_id !== wishId && e.type_id !== 'buckyos.cell' && e.type_id !== 'buckyos.wish' && e.type_id !== 'buckyos.block-def'
    && (e.type_id !== 'buckyos.container' || e.kind === 'folder') && store.outline.ancestors(e.entity_id).includes('data') && !SYSTEM_IDS.has(e.entity_id)
    && !store.outline.ancestors(e.entity_id).includes(SHOWS_ID) && !taken.some((i) => i.entity_id === e.entity_id))
}

export function WishInspector(context: RenderContext) {
  const wishId = context.source?.entity_id ?? ''
  if (!wishId) return null
  return <WishSettings wishId={wishId} mode={context.mode} />
}

function WishSettings({ wishId, mode }: { wishId: string; mode: RenderContext['mode'] }) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const { t } = useAiwsText()
  useOutlineVersion()
  const windowReason = useReadOnlyReason()
  const w = useWish(wishId)
  const invalidation = useInvalidation(w.payload, w.freshness)
  const [knowledge, setKnowledge] = useState<string | null>(null)
  const [refinement, setRefinement] = useState('')
  const [error, setError] = useState<string | null>(null)
  if (!w.payload) return w.readError ? <div className="aiws-error" role="alert">{w.readError}</div> : <div className="aiws-muted">{t('aiworkspace.wish.loading')}</div>
  const payload = w.payload
  const canEdit = mode === 'edit' && !windowReason && Boolean(w.entity?.capabilities.includes('update'))
  const inputs = payload.inputs ?? []
  const refinements = payload.refinements ?? []
  const title = (id: string) => { const e = store.outline.get(id); return e?.title ?? e?.name ?? id }
  const write = async (key: string, value: Json | null, label: string, asks: boolean) => {
    setError(null)
    if (asks && !(await invalidation.ask('request'))) return false
    try { await setWishKey(store, wishId, w.keyRevs, key, value, label); return true } catch (e) { setError(e instanceof Error ? e.message : String(e)); return false }
  }
  const setInputs = (next: WishInput[], label: string) => write('inputs', next as unknown as Json, label, true)
  const setRefinements = (next: WishRefinement[], label: string) => write('refinements', next.length ? (next as unknown as Json) : null, label, true)
  const startKnowledge = async (value: string, field: HTMLTextAreaElement) => {
    if (knowledge !== null) { setKnowledge(value); return }
    if (!(await invalidation.ask('request'))) return
    setKnowledge(value)
    field.focus()
  }
  const saveKnowledge = async () => {
    if (knowledge === null) return
    if (knowledge !== (payload.knowledge ?? '')) await write('knowledge', knowledge.trim() ? knowledge : null, t('aiworkspace.wish.edit.knowledge'), false)
    setKnowledge(null)
  }
  const modes = store.wish.executionModes()
  const candidates = canEdit ? inputCandidates(store, wishId, inputs) : []
  return (
    <div className="aiws-wp" data-testid="aiws-wish-props">
      <div className="aiws-wp-group">
        <div className="aiws-props-label">{t('aiworkspace.wish.inputs')}</div>
        {inputs.length === 0 && <div className="aiws-muted">{t('aiworkspace.wishcard.props.noInputs')}</div>}
        <ul className="aiws-wp-inputs">
          {inputs.map((input, index) => {
            const problem = w.freshness?.input_problems?.find((p) => p.entity_id === input.entity_id)
            return (
              <li key={`${input.entity_id}:${index}`} data-testid="aiws-wish-props-input" data-problem={problem?.reason ?? ''}>
                <button type="button" className="aiws-link" onClick={() => ui.openEntity(input.entity_id)}>{input.label ?? title(input.entity_id)}</button>
                <span className="aiws-muted">{scopeText(input, title, t)}{input.appended_by ? ` · ${t('aiworkspace.wish.appendedDuringRun')}` : ''}</span>
                {problem && <span className="aiws-error">{t(PROBLEM_LABEL[problem.reason] ?? PROBLEM_LABEL.missing)}</span>}
                {canEdit && (
                  <button type="button" className="aiws-tool aiws-tool-small" aria-label={t('aiworkspace.wish.removeInput')} title={t('aiworkspace.wish.removeInput')} data-testid="aiws-wish-props-remove-input"
                    onClick={() => { void setInputs(inputs.filter((_, i) => i !== index), t('aiworkspace.wish.removeInput')) }}><X size={14} /></button>
                )}
              </li>
            )
          })}
        </ul>
        {canEdit && (
          <select aria-label={t('aiworkspace.wish.addInput')} value="" data-testid="aiws-wish-props-add-input"
            onChange={(event) => { const id = event.target.value; if (id) void setInputs([...inputs, { entity_id: id, label: title(id), version: { mode: 'follow' } }], t('aiworkspace.wish.addInput')) }}>
            <option value="">{t('aiworkspace.wish.addInputOption')}</option>
            {candidates.map((e) => <option key={e.entity_id} value={e.entity_id}>{t('aiworkspace.wish.nameWithType', { name: e.title ?? e.name ?? e.entity_id, type: e.type_id.replace('buckyos.', '') })}</option>)}
          </select>
        )}
      </div>
      <div className="aiws-wp-group">
        <div className="aiws-props-label">{t('aiworkspace.wishcard.props.refinements')}</div>
        {refinements.map((r, index) => (
          <div key={`${index}:${r.text}`} className="aiws-wp-row" data-testid="aiws-wish-props-refinement">
            <input defaultValue={r.text} disabled={!canEdit} aria-label={t('aiworkspace.wishcard.props.refinements')}
              onBlur={(event) => {
                const value = event.target.value.trim()
                if (!value || value === r.text) { event.target.value = r.text; return }
                const input = event.target
                void setRefinements(refinements.map((x, i) => (i === index ? { ...x, text: value } : x)), t('aiworkspace.wish.edit.refinement')).then((ok) => { if (!ok) input.value = r.text })
              }} />
            {canEdit && (
              <button type="button" className="aiws-tool aiws-tool-small" aria-label={t('aiworkspace.wish.deleteRefinement')} title={t('aiworkspace.wish.deleteRefinement')} data-testid="aiws-wish-props-remove-refinement"
                onClick={() => { void setRefinements(refinements.filter((_, i) => i !== index), t('aiworkspace.wish.deleteRefinement')) }}><X size={14} /></button>
            )}
          </div>
        ))}
        {canEdit && (
          <form className="aiws-wp-row" onSubmit={(event) => {
            event.preventDefault()
            const value = refinement.trim()
            if (!value) return
            void setRefinements([...refinements, { text: value }], t('aiworkspace.wishcard.props.addRefinement')).then((ok) => { if (ok) setRefinement('') })
          }}>
            <input value={refinement} placeholder={t('aiworkspace.wishcard.props.refinementPlaceholder')} aria-label={t('aiworkspace.wishcard.props.addRefinement')} data-testid="aiws-wish-props-refinement-new" onChange={(event) => setRefinement(event.target.value)} />
            <button type="submit" data-testid="aiws-wish-props-add-refinement" disabled={!refinement.trim()}>{t('aiworkspace.wishcard.props.add')}</button>
          </form>
        )}
      </div>
      <label className="aiws-wp-group">
        <span className="aiws-props-label">{t('aiworkspace.wishcard.props.knowledge')}</span>
        <textarea rows={3} data-testid="aiws-wish-props-knowledge" placeholder={t('aiworkspace.wish.knowledgePlaceholder')} disabled={!canEdit}
          value={knowledge ?? payload.knowledge ?? ''} onChange={(event) => { void startKnowledge(event.target.value, event.target) }} onBlur={() => { void saveKnowledge() }} />
      </label>
      <label className="aiws-wp-group">
        <span className="aiws-props-label">{t('aiworkspace.wish.outputMode')}</span>
        <select data-testid="aiws-wish-props-output-mode" value={payload.output_mode ?? 'overwrite'} disabled={!canEdit}
          onChange={(event) => { void write('output_mode', event.target.value === 'new' ? 'new' : 'overwrite', t(event.target.value === 'new' ? 'aiworkspace.wish.edit.outputNew' : 'aiworkspace.wish.edit.outputOverwrite'), false) }}>
          <option value="overwrite">{t('aiworkspace.wishcard.props.outputOverwrite')}</option>
          <option value="new">{t('aiworkspace.wish.outputNew')}</option>
        </select>
      </label>
      {modes.length > 1 && (
        <label className="aiws-wp-group">
          <span className="aiws-props-label">{t('aiworkspace.wish.execution')}</span>
          <select data-testid="aiws-wish-props-exec-mode" value={store.wish.executionMode()} onChange={(event) => store.wish.setExecutionMode(event.target.value as WishExecutionMode)}>
            {modes.map((m) => <option key={m} value={m}>{t(EXECUTION_LABEL[m])}</option>)}
          </select>
        </label>
      )}
      {error && <div className="aiws-error" role="alert">{error}</div>}
      {invalidation.node}
    </div>
  )
}
