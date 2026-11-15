/* Shell panels shared by both top-level modes: the save-state list (the status panel's "需要处理", UI 框架 PRD §4.5). */

import { describeError } from '../../api/session'
import { EDIT_STATE_LABEL } from '../../state/edits'
import { useEdits, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'

/** Save states (design §6.5): entries that need attention, unsaved inputs, local-only saves. */
export function EditsPanel() {
  const store = useStore()
  const { t, tNode } = useAiwsText()
  const edits = useEdits()
  const list = [...edits.values()].sort((a, b) => b.at - a.at)
  const attention = list.filter((entry) => entry.state === 'needs_attention')
  const unsaved = list.filter((entry) => entry.state === 'unsaved')
  const local = list.filter((entry) => entry.state === 'saved_locally')
  if (attention.length === 0 && unsaved.length === 0 && local.length === 0) return null
  const replica = store.session.offline
  return (
    <div className="aiws-edits" data-testid="aiws-edits">
      {[...attention, ...unsaved, ...local].map((entry) => (
        <div key={entry.id} className={`aiws-edit aiws-edit-${entry.state}`} data-testid="aiws-edit-entry" data-state={entry.state} data-code={entry.code} data-edit-id={entry.id}>
          <span className={`aiws-state aiws-state-${entry.state}`}>{EDIT_STATE_LABEL[entry.state]}</span>
          <b>{entry.label}</b>
          {entry.detail && <div data-testid="aiws-edit-detail">{entry.detail}</div>}
          {entry.hasMine && <div>{tNode('aiworkspace.shell.edits.mine', { value: <code data-testid="aiws-edit-mine">{typeof entry.mine === 'string' ? entry.mine : JSON.stringify(entry.mine)}</code> })}</div>}
          {entry.state === 'needs_attention' && (
            <>
              {replica && entry.pendingKey && entry.code !== 'REVISION_CONFLICT' && entry.code !== 'TARGET_DELETED' && store.session.status().kind !== 'stopped' && (
                <button type="button" className="aiws-link" data-testid="aiws-edit-requeue" title={t('aiworkspace.shell.edits.requeueHint')}
                  onClick={() => { void replica.requeuePending(entry.pendingKey ?? '').catch((error: unknown) => store.notify('error', describeError(error))) }}>{t('aiworkspace.shell.edits.requeue')}</button>
              )}
              <button type="button" className="aiws-link" data-testid="aiws-edit-dismiss" onClick={() => { void store.dismissEdit(entry.id) }}>{entry.pendingKey ? t('aiworkspace.shell.edits.discard') : t('aiworkspace.shell.edits.dismiss')}</button>
            </>
          )}
          {entry.state === 'unsaved' && entry.storageFailed && (
            <button type="button" className="aiws-link" data-testid="aiws-edit-export" onClick={() => { void store.exportLocal() }}>{t('aiworkspace.shell.exportUnsaved')}</button>
          )}
        </div>
      ))}
    </div>
  )
}
