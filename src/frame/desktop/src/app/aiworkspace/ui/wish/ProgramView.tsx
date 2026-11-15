/* The macro's code (UI 框架 PRD §5.3; 许愿格交互升级 §3.4): line numbers in a monospace font, edited in place where the
 * user may. The data mode's "代码" page and the macro window of a wish's record show it. Starting to edit a macro that
 * has results first says the change makes them out of date (§3.5). */

import { useCallback, useState } from 'react'
import { describeError, type ReadOk } from '../../api/session'
import type { WishPayloadRead } from '../../api/types'
import { useFreshness, useLoad, useStore, useVersion } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { useInvalidation } from './confirm'
import './wish.css'

/** Code with its line numbers. */
export function CodeLines({ text }: { text: string }) {
  const lines = text.split('\n')
  return (
    <pre className="aiws-wish-code" data-testid="aiws-wish-code" data-lines={lines.length}>
      <code>{lines.map((line, i) => <span key={i} className="aiws-wish-code-line"><span className="aiws-wish-code-no" aria-hidden="true">{i + 1}</span>{line}{'\n'}</span>)}</code>
    </pre>
  )
}

/** The wish's saved macro, read at once and followed (a new coding run, another window's edit); `reason`: why it cannot be
 * edited, shown when it cannot. */
export function ProgramView({ wishId, canEdit, reason = null }: { wishId: string; canEdit: boolean; reason?: string | null }) {
  const store = useStore()
  const { t } = useAiwsText()
  const [draft, setDraft] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const version = useVersion(`e:${wishId}`)
  const loadSource = useCallback(() => store.wish.programSource(wishId).then((text) => text ?? ''), [store, wishId])
  const read = useLoad<string>(loadSource, version)
  const loadWish = useCallback(() => store.session.read<WishPayloadRead>(wishId), [store, wishId])
  const wish = useLoad<ReadOk<WishPayloadRead>>(loadWish, version)
  const freshness = useFreshness(wishId)
  const invalidation = useInvalidation(wish.data?.content.payload, freshness)
  const source = read.data ?? null
  const shownError = error ?? read.error
  const startEdit = async () => {
    if (source === null || !(await invalidation.ask('macro'))) return
    setDraft(source)
  }
  const save = async () => {
    if (draft === null) return
    try { await store.wish.saveProgram(wishId, draft); setDraft(null); read.reload() } catch (e) { setError(describeError(e)) }
  }
  if (source === null) return shownError ? <div className="aiws-error" role="alert">{shownError}</div> : <div className="aiws-muted">{t('aiworkspace.wish.loading')}</div>
  return (
    <div className="aiws-wish-program is-page" data-testid="aiws-wish-program-view" data-editable={canEdit ? 'true' : 'false'}>
      {!canEdit && reason && <div className="aiws-muted aiws-wish-program-reason" data-testid="aiws-wish-program-reason">{reason}</div>}
      {draft === null
        ? (source ? <CodeLines text={source} /> : <div className="aiws-muted">{t('aiworkspace.wish.program.none')}</div>)
        : <textarea className="aiws-wish-code-edit" data-testid="aiws-wish-program-edit" rows={24} value={draft} onChange={(e) => setDraft(e.target.value)} spellCheck={false} />}
      {shownError && <div className="aiws-error">{shownError}</div>}
      <div className="aiws-inline-form">
        {canEdit && draft === null && <button type="button" data-testid="aiws-wish-program-start-edit" onClick={() => { void startEdit() }}>{t('aiworkspace.common.edit')}</button>}
        {draft !== null && <button type="button" data-testid="aiws-wish-program-save" onClick={() => { void save() }}>{t('aiworkspace.wish.program.save')}</button>}
        {draft !== null && <button type="button" onClick={() => setDraft(null)}>{t('aiworkspace.wish.program.discard')}</button>}
      </div>
      {invalidation.node}
    </div>
  )
}
