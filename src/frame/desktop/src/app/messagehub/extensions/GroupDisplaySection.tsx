import { useState } from 'react'
import { ExternalLink, FileText, Star, Webhook } from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { getObjectAccess } from '../conversation/history/objectAccess'
import { hookServiceHost, type GroupDisplayInfo, type GroupHookInfo } from './model'

/**
 * The display configuration of a group or Session (EXT-04): announcement,
 * links, files and featured members. Nothing here grants anything; a
 * featured member is a mark, a file opens through the viewer's own object
 * access.
 */
export function GroupDisplaySection({ display, nameOf, testId = 'group-display' }: { display: GroupDisplayInfo; nameOf: (did: string) => string; testId?: string }) {
  const { t } = useI18n()
  if (!display.announcement && display.links.length === 0 && display.files.length === 0 && display.featuredMembers.length === 0) return null
  return <div className="space-y-2" data-testid={testId}>
    {display.announcement && <div className="rounded-xl bg-[color:color-mix(in_srgb,var(--cp-accent)_8%,transparent)] px-3 py-2 text-[13px]" data-testid="display-announcement"><p className="text-xs text-[color:var(--cp-muted)]">{t('messagehub.announcement')}</p><p className="whitespace-pre-wrap break-words">{display.announcement}</p></div>}
    {display.links.length > 0 && <ul className="space-y-1" data-testid="display-links">
      {display.links.map(link => <li key={`${link.title}\n${link.url}`}><a href={link.url} target="_blank" rel="noopener noreferrer" className="flex min-h-9 items-center gap-2 text-[13px] text-[color:var(--cp-accent)] hover:underline"><ExternalLink size={14} aria-hidden className="shrink-0" /><span className="min-w-0 truncate" title={link.url}>{link.title}</span></a></li>)}
    </ul>}
    {display.files.length > 0 && <ul className="space-y-1" data-testid="display-files">
      {display.files.map(file => <DisplayFile key={`${file.title}\n${file.objId}`} title={file.title} objId={file.objId} />)}
    </ul>}
    {display.featuredMembers.length > 0 && <div className="flex flex-wrap gap-1.5" data-testid="display-featured">
      {display.featuredMembers.map(did => <span key={did} title={did} className="flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px]" style={{ background: 'color-mix(in srgb, var(--cp-warning) 14%, transparent)', color: 'color-mix(in srgb, var(--cp-warning) 70%, var(--cp-text))' }}><Star size={11} aria-hidden />{nameOf(did)}</span>)}
    </div>}
  </div>
}

function DisplayFile({ title, objId }: { title: string; objId: string }) {
  const { t } = useI18n()
  const [state, setState] = useState<'idle' | 'opening' | 'failed'>('idle')
  const open = async () => {
    const access = getObjectAccess()
    if (!access) { setState('failed'); return }
    setState('opening')
    try {
      const url = await access.contentUrl(objId)
      window.open(url, '_blank', 'noopener')
      setState('idle')
    } catch {
      setState('failed')
    }
  }
  return <li className="flex min-h-9 items-center gap-2 text-[13px]">
    <FileText size={14} aria-hidden className="shrink-0 text-[color:var(--cp-muted)]" />
    <button type="button" className="min-w-0 truncate text-left text-[color:var(--cp-accent)] hover:underline disabled:opacity-60" disabled={state === 'opening'} title={objId} onClick={() => void open()}>{title}</button>
    {state === 'failed' && <span role="alert" className="shrink-0 text-xs text-[color:var(--cp-muted)]">{t('messagehub.display.fileUnavailable')}</span>}
  </li>
}

/** The business Hooks bound to the group (v2 §7.3): members see that a service decides, and for `post` reads what they write. */
export function GroupHooksSection({ hooks }: { hooks: readonly GroupHookInfo[] }) {
  const { t } = useI18n()
  if (hooks.length === 0) return null
  return <div className="rounded-xl px-3 py-2 text-[13px]" style={{ background: 'color-mix(in srgb, var(--cp-text) 4%, transparent)' }} data-testid="group-hooks">
    <p className="flex items-center gap-1.5 font-medium"><Webhook size={14} aria-hidden className="text-[color:var(--cp-muted)]" />{t('messagehub.hooks.count', undefined, { count: hooks.length })}</p>
    <ul className="mt-1 space-y-0.5 text-xs text-[color:var(--cp-muted)]">
      {hooks.map(hook => <li key={`${hook.point}\n${hook.service}`} className="break-all" data-point={hook.point}>{t(`messagehub.hooks.point.${hook.point}`, hook.point)} · {hookServiceHost(hook.service)}{hook.mayGrant ? ` · ${t('messagehub.hooks.mayGrant')}` : ''}</li>)}
    </ul>
    {hooks.some(hook => hook.point === 'post') && <p className="mt-1 text-xs text-[color:var(--cp-warning)]" data-testid="group-hooks-post-note">{t('messagehub.hooks.postReads')}</p>}
  </div>
}
