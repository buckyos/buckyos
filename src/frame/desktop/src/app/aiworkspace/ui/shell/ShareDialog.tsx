import { useCallback, useEffect, useId, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import FocusTrap from '@mui/material/Unstable_TrapFocus'
import { Check, Copy, Globe, Link2, LockKeyhole, Users, X } from 'lucide-react'
import { describeError } from '../../api/session'
import { useLoad, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { Modal } from './dialogs'
import { PUBLIC_LINK_CAPABILITIES, PUBLIC_SUBJECT, shareLink, useShell } from './shellContext'
import { ShareEntries } from './ShareEntries'

export function ShareDialog({ onClose }: { onClose: () => void }) {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  const id = useId()
  const [tab, setTab] = useState('link')
  const [copied, setCopied] = useState<'copied' | 'manual' | null>(null)
  const [tick, setTick] = useState(0)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.preventDefault()
      event.stopPropagation()
      onClose()
    }
    document.addEventListener('keydown', onKey, true)
    return () => document.removeEventListener('keydown', onKey, true)
  }, [onClose])
  const link = shareLink(store.session.info(), shell.topMode === 'canvas' ? shell.activeSurface : null)
  const where = shell.topMode === 'canvas' && shell.activeSurface ? t('aiworkspace.shell.share.whereCanvas', { name: shell.activeSurface.title ?? shell.activeSurface.name ?? '' }) : t('aiworkspace.shell.share.whereWorkspace')
  const canManage = store.session.info().capabilities.includes('manage')
  const loadGrants = useCallback(() => store.session.listGrants(), [store])
  const grants = useLoad(loadGrants, tick)
  const isPublic = (grants.data?.grants ?? []).some((g) => g.subject === PUBLIC_SUBJECT && !g.scope_entity_id)
  const copy = async () => {
    try { await navigator.clipboard.writeText(link); setCopied('copied') } catch { setCopied('manual') }
  }
  const change = (failedKey: string, work: () => Promise<void>) => {
    if (busy) return
    setBusy(true)
    setFailure(null)
    setCopied(null)
    work().then(copy).catch((error: unknown) => setFailure(t(failedKey, { error: describeError(error) }))).finally(() => { setBusy(false); setTick((n) => n + 1) })
  }
  const tabs = [
    { key: 'link', label: t('aiworkspace.shell.share.link'), icon: Link2 },
    { key: 'entries', label: t('aiworkspace.shell.share.entries'), icon: Users },
  ]
  const switchTab = (event: ReactKeyboardEvent<HTMLButtonElement>, index: number) => {
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : event.key === 'ArrowRight' ? (index + 1) % tabs.length : event.key === 'ArrowLeft' ? (index + tabs.length - 1) % tabs.length : null
    if (next === null) return
    event.preventDefault()
    setTab(tabs[next].key)
    event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus()
  }
  return (
    <Modal label={t('aiworkspace.shell.share.label')} testId="aiws-share-panel" className="aiws-share-dialog" onClose={onClose}>
      <FocusTrap open getTabbable={(root) => Array.from(root.querySelectorAll<HTMLElement>('button, input, select, [tabindex]')).filter((element) => element.tabIndex >= 0 && !element.matches(':disabled') && !element.closest('[hidden]'))}>
        <div className="aiws-share-dialog-layout" tabIndex={-1} onKeyDown={(event) => { if (event.key !== 'Escape') event.stopPropagation() }}>
          <div className="aiws-share-dialog-head">
            <div className="aiws-share-heading">
              <h2>{t('aiworkspace.shell.share.button')}</h2>
              <p className="aiws-muted">{t('aiworkspace.shell.share.destination', { where })}</p>
            </div>
            <button type="button" className="aiws-tool" aria-label={t('aiworkspace.common.close')} data-testid="aiws-share-close" onClick={onClose}><X size={20} /></button>
          </div>
          {canManage && (
            <div className="aiws-share-tabs" role="tablist" aria-label={t('aiworkspace.shell.share.label')}>
              {tabs.map(({ key, label, icon: Icon }, index) => (
                <button key={key} type="button" role="tab" id={`${id}-${key}-tab`} aria-controls={`${id}-${key}-panel`} aria-selected={tab === key} tabIndex={tab === key ? 0 : -1} data-testid={`aiws-share-tab-${key}`} onClick={() => setTab(key)} onKeyDown={(event) => switchTab(event, index)}>
                  <Icon size={16} aria-hidden="true" />{label}
                </button>
              ))}
            </div>
          )}
          <div className="aiws-share-dialog-body">
            <div id={`${id}-link-panel`} role={canManage ? 'tabpanel' : undefined} aria-labelledby={canManage ? `${id}-link-tab` : undefined} hidden={tab !== 'link'}>
              <div className="aiws-share-link-content">
                <section className="aiws-share-section">
                  <h3>{t('aiworkspace.shell.share.link')}</h3>
                  <div className="aiws-share-link-row">
                    <input readOnly aria-label={t('aiworkspace.shell.share.link')} value={link} data-testid="aiws-share-link" onFocus={(event) => event.target.select()} />
                    <button type="button" className="is-primary" data-testid="aiws-share-copy" onClick={() => { void copy() }}>
                      {copied === 'copied' ? <Check size={16} aria-hidden="true" /> : <Copy size={16} aria-hidden="true" />}{t('aiworkspace.shell.share.copyTitle')}
                    </button>
                  </div>
                  {copied && <div className="aiws-muted" role="status">{copied === 'manual' ? t('aiworkspace.shell.share.copyManually') : t(isPublic ? 'aiworkspace.shell.share.copiedPublic' : 'aiworkspace.shell.share.copied')}</div>}
                </section>
                {grants.data && (
                  <section className={`aiws-share-access${isPublic ? ' is-public' : ''}`} data-testid="aiws-share-public-state" data-public={isPublic ? 'true' : 'false'}>
                    <div className="aiws-share-access-title">
                      {isPublic ? <Globe size={18} aria-hidden="true" /> : <LockKeyhole size={18} aria-hidden="true" />}
                      <h3>{t(isPublic ? 'aiworkspace.shell.share.publicTitle' : 'aiworkspace.shell.share.privateTitle')}</h3>
                    </div>
                    <p className="aiws-muted">{t(isPublic ? 'aiworkspace.shell.share.publicOn' : 'aiworkspace.shell.share.privateNote', { where })}</p>
                    {canManage && (isPublic
                      ? <button type="button" data-testid="aiws-share-unpublish" disabled={busy || grants.loading} onClick={() => change('aiworkspace.shell.share.unpublishFailed', () => store.session.revoke(PUBLIC_SUBJECT))}>{t('aiworkspace.shell.share.unpublish')}</button>
                      : <button type="button" data-testid="aiws-share-publish" disabled={busy || grants.loading} title={t('aiworkspace.shell.share.publishHint')} onClick={() => change('aiworkspace.shell.share.publishFailed', () => store.session.grant(PUBLIC_SUBJECT, [...PUBLIC_LINK_CAPABILITIES]))}><Globe size={16} aria-hidden="true" />{t('aiworkspace.shell.share.publish')}</button>)}
                  </section>
                )}
                {grants.loading && !grants.data && <div className="aiws-muted" role="status">{t('aiworkspace.shell.share.loading')}</div>}
                {grants.error && <div className="aiws-error" role="alert">{grants.error}</div>}
                {failure && <div className="aiws-error" role="alert" data-testid="aiws-share-error">{failure}</div>}
                {canManage && <button type="button" className="aiws-share-collab" data-testid="aiws-share-collab" onClick={() => { onClose(); shell.setSide('collab') }}><Users size={16} aria-hidden="true" />{t('aiworkspace.shell.share.grantInCollab')}</button>}
              </div>
            </div>
            {canManage && <div id={`${id}-entries-panel`} role="tabpanel" aria-labelledby={`${id}-entries-tab`} hidden={tab !== 'entries'}><ShareEntries client={shell.client} workspaceId={store.session.workspaceId} /></div>}
          </div>
          {!shell.identity.guest && <div className="aiws-share-dialog-foot aiws-muted">{t('aiworkspace.shell.share.exportHint')}</div>}
        </div>
      </FocusTrap>
    </Modal>
  )
}
