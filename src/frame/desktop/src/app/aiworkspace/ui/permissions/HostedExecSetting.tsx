/* The owner's hosted execution (内核架构改进 D22, T3-7), a section of the permission editor: which Agent runs tasks for
 * collaborators who may only `run` a wish, which wishes, and who may trigger them. Saving it is the owner's (R17). */

import { useState } from 'react'
import { describeError } from '../../api/session'
import type { HostedExecConfig } from '../../api/types'
import { useOutlineVersion, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'

/** The owner's hosted execution (内核架构改进 D22, T3-7): which Agent runs tasks for collaborators who may only `run`
 * a wish, which wishes, and who may trigger them. Saving changes the configuration digest: runs accepted before it
 * keep their binding, a later apply of theirs is DELEGATION_STALE. */
export function HostedExecSetting({ online }: { online: boolean }) {
  const { t } = useAiwsText()
  const store = useStore()
  useOutlineVersion()
  const current = store.session.info().hosted_exec
  const [draft, setDraft] = useState<HostedExecConfig>(() => ({ enabled: current?.enabled ?? false, agent_id: current?.agent_id ?? '', allowed_wishes: current?.allowed_wishes ?? [], triggerers: current?.triggerers ?? [] }))
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  // the owner's own operation (权限需求 R17): it uses the owner's Agent and AI
  const canManage = store.session.info().owner === true
  const wishes = store.outline.all().filter((e) => e.type_id === 'buckyos.wish' && !e.deleted)
  const dirty = JSON.stringify(draft) !== JSON.stringify({ enabled: current?.enabled ?? false, agent_id: current?.agent_id ?? '', allowed_wishes: current?.allowed_wishes ?? [], triggerers: current?.triggerers ?? [] })
  const save = () => {
    setBusy(true)
    setFailure(null)
    setSaved(false)
    const config: HostedExecConfig = { enabled: draft.enabled, agent_id: draft.agent_id?.trim() || undefined, allowed_wishes: draft.allowed_wishes?.length ? draft.allowed_wishes : undefined, triggerers: draft.triggerers?.length ? draft.triggerers : undefined }
    store.session.setHostedExec(config)
      .then(() => setSaved(true))
      .catch((error: unknown) => setFailure(t('aiworkspace.data.perm.hostedFailed', { error: describeError(error) })))
      .finally(() => setBusy(false))
  }
  return (
    <div className="aiws-wish-section" data-testid="aiws-hosted-exec" data-enabled={current?.enabled ? 'true' : 'false'}>
      <label className="aiws-inline-form">
        <input type="checkbox" data-testid="aiws-hosted-enabled" checked={draft.enabled} disabled={!canManage || !online || busy} onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })} />
        {t('aiworkspace.data.perm.hosted')}
      </label>
      <div className="aiws-muted">{t('aiworkspace.data.perm.hostedHint')}</div>
      {draft.enabled && (
        <>
          <label className="aiws-inline-form">{t('aiworkspace.data.perm.hostedAgent')}
            <input data-testid="aiws-hosted-agent" value={draft.agent_id ?? ''} disabled={!canManage || !online || busy} placeholder="jarvis.<zone>" onChange={(e) => setDraft({ ...draft, agent_id: e.target.value })} />
          </label>
          <label className="aiws-inline-form">{t('aiworkspace.data.perm.hostedTriggerers')}
            <input data-testid="aiws-hosted-triggerers" value={(draft.triggerers ?? []).join(', ')} disabled={!canManage || !online || busy} placeholder={t('aiworkspace.data.perm.hostedTriggerersAll')} onChange={(e) => setDraft({ ...draft, triggerers: e.target.value.split(',').map((s) => s.trim()).filter(Boolean) })} />
          </label>
          <div className="aiws-muted">{t('aiworkspace.data.perm.hostedWishes')}</div>
          {wishes.length === 0 && <div className="aiws-muted">{t('aiworkspace.data.perm.hostedNoWishes')}</div>}
          {wishes.map((w) => (
            <label key={w.entity_id} className="aiws-inline-form">
              <input type="checkbox" data-testid={`aiws-hosted-wish-${w.entity_id}`} checked={(draft.allowed_wishes ?? []).includes(w.entity_id)} disabled={!canManage || !online || busy}
                onChange={(e) => setDraft({ ...draft, allowed_wishes: e.target.checked ? [...(draft.allowed_wishes ?? []), w.entity_id] : (draft.allowed_wishes ?? []).filter((id) => id !== w.entity_id) })} />
              {w.title ?? w.name ?? w.entity_id}
            </label>
          ))}
          <div className="aiws-muted">{t('aiworkspace.data.perm.hostedWishesHint')}</div>
        </>
      )}
      <div className="aiws-inline-form">
        <button type="button" data-testid="aiws-hosted-save" disabled={!canManage || !online || busy || !dirty} onClick={save}>{t('aiworkspace.common.save')}</button>
        {saved && !dirty && <span className="aiws-muted">{t('aiworkspace.data.perm.hostedSaved')}</span>}
      </div>
      {failure && <div className="aiws-error" role="alert">{failure}</div>}
    </div>
  )
}
