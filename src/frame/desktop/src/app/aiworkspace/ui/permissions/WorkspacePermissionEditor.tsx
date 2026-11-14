/* The Workspace's permission editor (UI 框架 PRD §8): the groups (fixed first, then the logical groups with their members
 * and rules), the rule list, "check permissions", the state of anonymous access and share entries (changed in the share
 * dialog), the owner's hosted execution; the operation record when no sidebar shows it. On a phone or a narrow panel each
 * part is a screen of its own, entered one level at a time (§8.5). Without `manage` it is "my permissions" (§5.2).
 * Meant as the content of a tab of the data mode. */

import { useEffect, useRef, useState, type ReactNode, type RefObject } from 'react'
import { ChevronLeft, ChevronRight, Globe, Link2, LockKeyhole } from 'lucide-react'
import type { AclEntryList, ShareEntry } from '../../api/types'
import { useAiwsText } from '../../useAiwsText'
import { usePhone, useShell } from '../shell/shellContext'
import { useAcl, useAclRead, useWorkspaceCaps } from './aclData'
import { AclAuditList } from './AclAuditList'
import { CheckPanel } from './CheckPanel'
import { GroupDetail, GroupList, type GroupsData } from './GroupsPanel'
import { HostedExecSetting } from './HostedExecSetting'
import { MyPermissions } from './MyPermissions'
import { RulesPanel } from './RulesPanel'

export interface WorkspacePermissionEditorProps {
  /** Show the operation record as a part of the editor (when no sidebar of the host shows `AclAuditList`). */
  audit?: boolean
}

type Part = 'groups' | 'rules' | 'check' | 'audit' | 'hosted'

/** Narrow: a phone, or a container below 640 px (the side panel, a drawer). */
function useNarrow(): [RefObject<HTMLDivElement | null>, boolean] {
  const phone = usePhone()
  const ref = useRef<HTMLDivElement | null>(null)
  const [narrow, setNarrow] = useState(false)
  useEffect(() => {
    const el = ref.current
    if (!el || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(() => setNarrow(el.clientWidth > 0 && el.clientWidth < 640))
    observer.observe(el)
    return () => observer.disconnect()
  }, [])
  return [ref, phone || narrow]
}

/** Anonymous access and the share entries (§8.5): their state here, their switches in the share dialog. */
export function AccessStatus({ all, shares }: { all: AclEntryList; shares: ShareEntry[] | undefined }) {
  const { t } = useAiwsText()
  const shell = useShell()
  const open = (shares ?? []).filter((e) => e.state === 'active')
  return (
    <div className="aiws-perm-access" data-testid="aiws-perm-access">
      <span className={`aiws-perm-access-item${all.anonymous ? ' is-on' : ''}`} data-testid="aiws-perm-anonymous" data-on={all.anonymous ? 'true' : 'false'}>
        {all.anonymous ? <Globe size={16} aria-hidden="true" /> : <LockKeyhole size={16} aria-hidden="true" />}
        {all.anonymous ? t('aiworkspace.perm.access.anonymousOn') : t('aiworkspace.perm.access.anonymousOff')}
      </span>
      {shares && (
        <span className="aiws-perm-access-item" data-testid="aiws-perm-share-entries" data-count={open.length}>
          <Link2 size={16} aria-hidden="true" />{t('aiworkspace.perm.access.entries', { count: open.length })}
        </span>
      )}
      <button type="button" className="aiws-link" data-testid="aiws-perm-open-share" onClick={() => shell.openDialog({ kind: 'share' })}>{t('aiworkspace.perm.access.openShare')}</button>
    </div>
  )
}

export function WorkspacePermissionEditor({ audit = false }: WorkspacePermissionEditorProps) {
  const { capabilities } = useWorkspaceCaps()
  if (!capabilities.includes('manage')) {
    return <div className="aiws-perm-editor" data-testid="aiws-permissions" data-complete="false"><MyPermissions /></div>
  }
  return <ManagerEditor audit={audit} />
}

function ManagerEditor({ audit }: { audit: boolean }) {
  const { t } = useAiwsText()
  const { api, online } = useAcl()
  const { owner } = useWorkspaceCaps()
  const [ref, narrow] = useNarrow()
  const all = useAclRead('all', api.all)
  const groups = useAclRead('groups', api.groups)
  const shares = useAclRead('share-entries', api.entries)
  const [part, setPart] = useState<Part | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  const parts: Part[] = ['groups', 'rules', 'check', ...(audit ? ['audit' as const] : []), 'hosted']
  const current: Part | null = narrow ? part : part ?? 'groups'
  const data: GroupsData | null = all.data && groups.data ? { all: all.data, groups: groups.data.groups, entries: shares.data ?? [], owner } : null
  const entryTargets = new Set((shares.data ?? []).filter((e) => e.state === 'active').map((e) => `@${e.group_id}`))
  const label: Record<Part, string> = {
    groups: t('aiworkspace.perm.part.groups'), rules: t('aiworkspace.perm.part.rules'), check: t('aiworkspace.perm.part.check'),
    audit: t('aiworkspace.perm.part.audit'), hosted: t('aiworkspace.perm.part.hosted'),
  }
  const body = (p: Part): ReactNode => {
    if (!data) return null
    switch (p) {
      case 'groups':
        if (narrow) return selected ? <GroupDetail key={selected} subject={selected} data={data} onDeleted={() => setSelected(null)} /> : <GroupList data={data} selected={null} onSelect={setSelected} />
        return (
          <div className="aiws-perm-groups-layout">
            <GroupList data={data} selected={selected} onSelect={setSelected} />
            <div className="aiws-perm-groups-detail">
              {selected ? <GroupDetail key={selected} subject={selected} data={data} onDeleted={() => setSelected(null)} /> : <div className="aiws-muted" data-testid="aiws-perm-group-pick">{t('aiworkspace.perm.group.pick')}</div>}
            </div>
          </div>
        )
      case 'rules': return <RulesPanel all={data.all} entryTargets={entryTargets} owner={owner} />
      case 'check': return <CheckPanel />
      case 'audit': return <AclAuditList />
      case 'hosted': return <HostedExecSetting online={online} />
    }
  }
  const back = () => { if (current === 'groups' && selected) setSelected(null); else setPart(null) }
  return (
    <div ref={ref} className={`aiws-perm-editor${narrow ? ' is-narrow' : ''}`} data-testid="aiws-permissions" data-complete="true" data-narrow={narrow ? 'true' : 'false'} data-part={current ?? 'home'}>
      <div className="aiws-perm-editor-head">
        {narrow && current && <button type="button" className="aiws-tool" aria-label={t('aiworkspace.perm.back')} data-testid="aiws-perm-back" onClick={back}><ChevronLeft size={20} /></button>}
        <h2>{narrow && current ? label[current] : t('aiworkspace.perm.title')}</h2>
      </div>
      {!online && <div className="aiws-warning" data-testid="aiws-perm-offline">{t('aiworkspace.perm.offline')}</div>}
      {(all.error || groups.error) && <div className="aiws-error" role="alert">{all.error ?? groups.error}</div>}
      {all.data && (!narrow || !current) && <AccessStatus all={all.data} shares={shares.data} />}
      {!data && !all.error && !groups.error && <div className="aiws-muted">{t('aiworkspace.perm.loading')}</div>}
      {data && !narrow && (
        <div className="aiws-perm-tabs" role="tablist" aria-label={t('aiworkspace.perm.title')}>
          {parts.map((p) => <button key={p} type="button" role="tab" aria-selected={current === p} data-testid={`aiws-perm-tab-${p}`} onClick={() => setPart(p)}>{label[p]}</button>)}
        </div>
      )}
      {data && narrow && !current && (
        <nav className="aiws-perm-nav" aria-label={t('aiworkspace.perm.title')}>
          {parts.map((p) => (
            <button key={p} type="button" className="aiws-perm-nav-item" data-testid={`aiws-perm-tab-${p}`} onClick={() => { setPart(p); setSelected(null) }}>
              <span>{label[p]}</span>
              <span className="aiws-muted">{p === 'groups' ? t('aiworkspace.perm.groupsCount', { count: data.groups.length + 4 }) : p === 'rules' ? t('aiworkspace.perm.rulesCount', { count: data.all.entries.length }) : ''}</span>
              <ChevronRight size={18} aria-hidden="true" />
            </button>
          ))}
        </nav>
      )}
      {data && current && <div className="aiws-perm-part" role={narrow ? undefined : 'tabpanel'} data-testid={`aiws-perm-part-${current}`}>{body(current)}</div>}
    </div>
  )
}
