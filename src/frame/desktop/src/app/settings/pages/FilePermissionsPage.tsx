import { Button } from '@mui/material'
import { RefreshCw } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { installFileBrowserData } from '../../filebrowser/data/install'
import { permissionSource } from '../../filebrowser/data/permissions'
import type { AclRecordList, AclRecordView, PrincipalOption } from '../../filebrowser/data/permissions'
import { toUiError } from '../../filebrowser/data/state'
import { PermissionsDialog } from '../../filebrowser/dialogs/PermissionsDialog'
import { levelLabel, principalLabel } from '../../filebrowser/dialogs/permissionLabels'
import { ListCard, Section, SectionNote, StatusBadge } from '../components/shared/Section'
import { formatDateTime } from '../components/shared/format'

const SHARED_GROUP = ''

/** Every path ACL the caller manages — for the zone owner, every user's settings. */
export function FilePermissionsPage() {
  const { t, locale } = useI18n()
  const [list, setList] = useState<AclRecordList | null>(null)
  const [users, setUsers] = useState<PrincipalOption[]>([])
  const [error, setError] = useState<string | null>(null)
  const [filter, setFilter] = useState('')
  const [editing, setEditing] = useState<AclRecordView | null>(null)
  const [reload, setReload] = useState(0)

  useEffect(() => {
    installFileBrowserData()
    const source = permissionSource()
    if (!source) return
    let alive = true
    source.listAcls().then((result) => { if (alive) { setList(result); setError(null) } }).catch((err: unknown) => { if (alive) setError(toUiError(err).fallback) })
    source.listUsers().then((result) => { if (alive) setUsers(result) }).catch(() => {})
    return () => { alive = false }
  }, [reload])

  const groups = useMemo(() => {
    const needle = filter.trim().toLowerCase()
    const matches = (record: AclRecordView) => !needle
      || record.path.toLowerCase().includes(needle)
      || record.entries.some((entry) => entry.principal.toLowerCase().includes(needle) || principalLabel(t, entry.principal, users).toLowerCase().includes(needle))
    const byOwner = new Map<string, AclRecordView[]>()
    for (const record of list?.records ?? []) {
      if (!matches(record)) continue
      const key = record.homeOwner ?? SHARED_GROUP
      byOwner.set(key, [...(byOwner.get(key) ?? []), record])
    }
    return [...byOwner.entries()].sort(([a], [b]) => (a === SHARED_GROUP ? -1 : b === SHARED_GROUP ? 1 : a.localeCompare(b)))
  }, [list, filter, users, t])

  const removeOrphan = async (record: AclRecordView) => {
    try {
      await permissionSource()?.setAcl(record.path, { inherit: true, entries: [] }, record.revision)
      setReload((n) => n + 1)
    } catch (err) {
      setError(toUiError(err).fallback)
    }
  }

  const groupTitle = (owner: string) => owner === SHARED_GROUP
    ? t('settings.filePermissions.sharedAreas', 'Shared areas')
    : t('settings.filePermissions.homeOf', 'Home of {{name}}', { name: principalLabel(t, `user:${owner}`, users) })

  return (
    <div className="space-y-4" data-testid="settings-file-permissions-page">
      <Section
        title={t('settings.filePermissions.title', 'File permissions')}
        description={list?.isAdmin
          ? t('settings.filePermissions.adminDescription', 'Everything shared in this zone, by every user. The zone owner always has full control, and every user always has full control of their own home folder.')
          : t('settings.filePermissions.userDescription', 'Files and folders you manage that give other people access.')}
        actions={<Button size="small" startIcon={<RefreshCw size={14} />} onClick={() => setReload((n) => n + 1)}>{t('settings.filePermissions.refresh', 'Refresh')}</Button>}
        testId="settings-file-permissions"
      >
        <input
          aria-label={t('settings.filePermissions.filter', 'Filter by path or person')}
          placeholder={t('settings.filePermissions.filter', 'Filter by path or person')}
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          className="mb-3 w-full rounded border bg-transparent p-2 text-sm"
        />
        {!permissionSource() && <SectionNote>{t('settings.filePermissions.unavailable', 'The file service is not available.')}</SectionNote>}
        {error && <SectionNote tone="danger">{error}</SectionNote>}
        {!list && !error && permissionSource() && <SectionNote>{t('settings.filePermissions.loading', 'Loading…')}</SectionNote>}
        {list && groups.length === 0 && <SectionNote>{filter ? t('settings.filePermissions.noMatch', 'No permissions match this filter.') : t('settings.filePermissions.empty', 'Nothing is shared yet. Open Files, right-click a file or folder and choose Permissions.')}</SectionNote>}
        <div className="space-y-4">
          {groups.map(([owner, records]) => (
            <div key={owner || 'shared'} data-testid={`file-permissions-group-${owner || 'shared'}`}>
              <h4 className="mb-2 text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{groupTitle(owner)}</h4>
              <div className="space-y-2">
                {records.map((record) => (
                  <ListCard key={record.path} testId={`file-permission-${record.path}`}>
                    <div className="flex flex-wrap items-start justify-between gap-2">
                      <div className="min-w-0 flex-1">
                        <p className="break-all font-mono text-sm" style={{ color: 'var(--cp-text)' }}>{record.path}</p>
                        <div className="mt-1 flex flex-wrap gap-1.5">
                          {!record.inherit && <StatusBadge status="info" label={t('settings.filePermissions.notInherited', 'Inheritance off')} />}
                          {record.missing && <StatusBadge status="warn" label={t('settings.filePermissions.missing', 'Path no longer exists')} />}
                          {record.entries.map((entry) => (
                            <StatusBadge key={entry.principal} status={entry.principal === 'public' ? 'warn' : 'neutral'} label={`${principalLabel(t, entry.principal, users)} · ${levelLabel(t, entry.level)}`} />
                          ))}
                          {record.entries.length === 0 && <StatusBadge status="neutral" label={t('settings.filePermissions.noEntries', 'Only built-in access')} />}
                        </div>
                        <p className="mt-1 text-xs" style={{ color: 'var(--cp-muted)' }}>
                          {t('settings.filePermissions.updated', 'Changed by {{who}} · {{when}}', { who: principalLabel(t, record.updatedBy, users), when: formatDateTime(record.updatedAt, locale) })}
                        </p>
                      </div>
                      <div className="flex shrink-0 gap-1">
                        {record.missing
                          ? <Button size="small" color="error" onClick={() => void removeOrphan(record)}>{t('settings.filePermissions.remove', 'Remove')}</Button>
                          : <Button size="small" onClick={() => setEditing(record)} data-testid={`file-permission-edit-${record.path}`}>{t('settings.filePermissions.edit', 'Edit')}</Button>}
                      </div>
                    </div>
                  </ListCard>
                ))}
              </div>
            </div>
          ))}
        </div>
      </Section>
      {editing && (
        <PermissionsDialog
          key={editing.path}
          request={{ path: editing.path, name: editing.path.slice(editing.path.lastIndexOf('/') + 1) || editing.path, onSaved: () => setReload((n) => n + 1) }}
          onClose={() => setEditing(null)}
        />
      )}
    </div>
  )
}
