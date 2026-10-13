import { Bot, FolderLock, Globe, Inbox, LayoutGrid, Lock, MessageCircle, RefreshCw, ShieldBan, Users } from 'lucide-react'
import { IconButton, Tooltip } from '@mui/material'
import type { ReactNode } from 'react'
import { useI18n } from '../../../i18n/provider'
import { pickLocalized } from '../../../presentation/model'
import type { LocalizedText } from '../../../presentation/model'
import { useSettingsResource, useSettingsStore } from '../hooks/use-settings-store'
import { ListCard, Section, SectionNote, StatusBadge } from '../components/shared/Section'
import type { AppAccessEntry, AppAudience, AppMount, PrivacyOverview, Section as DataSection } from '../data/privacy'

type Translate = ReturnType<typeof useI18n>['t']

/** Friendly names for the permission scopes of buckyos-api `permission.rs` / AppDoc defaults. */
const permissionLabels: Record<string, [string, string]> = {
  'kapi/aicc': ['settings.privacy.perm.aicc', 'AI models'],
  'kapi/task-manager': ['settings.privacy.perm.taskManager', 'Background tasks'],
  'kapi/workflow': ['settings.privacy.perm.workflow', 'Workflows'],
  'kapi/msg-center': ['settings.privacy.perm.msgCenter', 'Messages'],
  'kapi/kmsg': ['settings.privacy.perm.kmsg', 'Message queue'],
  'kapi/repo-service': ['settings.privacy.perm.repo', 'Package repository'],
  wan: ['settings.privacy.perm.wan', 'Internet access'],
  lan: ['settings.privacy.perm.lan', 'Local network access'],
  'access/all': ['settings.privacy.perm.all', 'Full system access'],
  'devices/iot': ['settings.privacy.perm.iot', 'IoT devices'],
  'user/home': ['settings.privacy.perm.home', 'Your home folder'],
  'zone/library': ['settings.privacy.perm.library', 'Zone library'],
  'zone/public': ['settings.privacy.perm.public', 'Zone public data'],
  'zone/location': ['settings.privacy.perm.location', 'Location'],
  'fs.data': ['settings.privacy.perm.fsData', 'Its own data folder'],
  'fs.cache': ['settings.privacy.perm.fsCache', 'Its own cache folder'],
  'fs.library': ['settings.privacy.perm.fsLibrary', 'Zone library'],
}

function permissionLabel(scope: string, t: Translate): string {
  const entry = permissionLabels[scope]
  return entry ? t(entry[0], entry[1]) : scope
}

function accessLabel(access: string, t: Translate): string {
  switch (access) {
    case 'read_write':
      return t('settings.privacy.access.readWrite', 'read & write')
    case 'read_write_append':
      return t('settings.privacy.access.readAppend', 'read & append')
    default:
      return t('settings.privacy.access.readOnly', 'read only')
  }
}

/** The host folder of a mount, relative to the data root when it lives there. */
function mountTarget(mount: AppMount): string {
  const target = mount.target.replace(/\\/g, '/')
  const index = target.indexOf('/data/')
  if (target.startsWith('/') && index >= 0) return target.slice(index + '/data'.length)
  return target ? (target.startsWith('/') ? target : `/${target}`) : mount.path
}

function audienceBadge(audience: AppAudience, sharedUsers: string[], t: Translate) {
  switch (audience) {
    case 'public':
      return <StatusBadge status="warn" label={t('settings.privacy.audience.public', 'Anyone, without signing in')} />
    case 'users':
      return <StatusBadge status="info" label={t('settings.privacy.audience.users', 'All Zone users')} />
    case 'admins':
      return <StatusBadge status="info" label={t('settings.privacy.audience.admins', 'Administrators')} />
    case 'selected':
      return <StatusBadge status="info" label={t('settings.privacy.audience.selected', 'You and {{count}} users', { count: sharedUsers.length })} />
    default:
      return <StatusBadge status="pass" label={t('settings.privacy.audience.owner', 'Only the owner')} />
  }
}

function SectionState<T>({ value, children, empty }: { value: DataSection<T> | null; children: (data: T) => ReactNode; empty?: ReactNode }) {
  const { t } = useI18n()
  if (!value) return empty ?? null
  if (!value.ok) return <SectionNote tone="danger">{t('settings.privacy.sourceUnavailable', 'Could not be read: {{detail}}', { detail: value.error })}</SectionNote>
  return <>{children(value.data)}</>
}

function ExposureRow({ icon, title, detail, url, badge }: { icon: ReactNode; title: string; detail: ReactNode; url?: string; badge?: ReactNode }) {
  return (
    <ListCard>
      <div className="flex items-start gap-3">
        <span className="mt-0.5 shrink-0" style={{ color: 'var(--cp-muted)' }}>{icon}</span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{title}</span>
            {badge}
          </div>
          {url ? (
            <a className="mt-1 block break-all font-mono text-xs underline-offset-2 hover:underline" style={{ color: 'var(--cp-accent)' }} href={url} target="_blank" rel="noreferrer">
              {url}
            </a>
          ) : null}
          <div className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>{detail}</div>
        </div>
      </div>
    </ListCard>
  )
}

function PublicAccessSection({ data, actions }: { data: PrivacyOverview; actions: ReactNode }) {
  const { t, locale } = useI18n()
  const name = (text: LocalizedText) => pickLocalized(text, locale) ?? '—'
  const publicBadge = <StatusBadge status="warn" label={t('settings.privacy.public', 'Public')} />

  return (
    <Section title={t('settings.privacy.publicAccess', 'Public Access')} testId="settings-privacy-public" actions={actions}>
      <div className="space-y-2">
        <ExposureRow
          icon={<Globe size={16} />}
          title={t('settings.privacy.signInPage', 'Sign-in page')}
          url={`https://${data.zoneHost}/login`}
          badge={publicBadge}
          detail={t('settings.privacy.signInPageDesc', 'Anyone can see the Zone name, description and icon here.')}
        />
        <SectionState value={data.publicApps}>
          {(apps) =>
            apps.length > 0 ? (
              apps.map((app) => (
                <ExposureRow
                  key={app.appInstanceId}
                  icon={<LayoutGrid size={16} />}
                  title={name(app.name)}
                  url={app.urls[0]}
                  badge={publicBadge}
                  detail={t('settings.privacy.publicAppDesc', 'This app is open to visitors without signing in.')}
                />
              ))
            ) : (
              <SectionNote>{t('settings.privacy.noPublicApps', 'None of your apps is open to visitors without signing in.')}</SectionNote>
            )
          }
        </SectionState>
        <SectionState value={data.homestation}>
          {(home) => (
            <ExposureRow
              icon={<Users size={16} />}
              title={t('settings.privacy.homestation', 'HomeStation')}
              url={home.url}
              badge={home.publicEntries > 0 ? publicBadge : undefined}
              detail={
                <>
                  {home.publicEntries > 0
                    ? t('settings.privacy.homestationPublic', '{{count}} posts are visible to anyone.', { count: home.more ? `${home.publicEntries}+` : home.publicEntries })
                    : t('settings.privacy.homestationNoPublic', 'Nothing is visible to visitors yet.')}{' '}
                  {t('settings.privacy.homestationDefault', 'New posts go to: {{audience}}.', { audience: t(`settings.privacy.hsAudience.${home.defaultAudience}`, home.defaultAudience) })}
                </>
              }
            />
          )}
        </SectionState>
        <SectionState value={data.workspaces}>
          {(workspaces) =>
            workspaces.length > 0 ? (
              workspaces.map((workspace) => (
                <ExposureRow
                  key={workspace.workspaceId}
                  icon={<LayoutGrid size={16} />}
                  title={workspace.title || workspace.workspaceId}
                  url={workspace.url}
                  badge={publicBadge}
                  detail={t('settings.privacy.publicWorkspaceDesc', 'AI Workspace with a public link: anyone with the link can open it.')}
                />
              ))
            ) : (
              <SectionNote>{t('settings.privacy.noPublicWorkspaces', 'No AI Workspace has a public link.')}</SectionNote>
            )
          }
        </SectionState>
      </div>
    </Section>
  )
}

function MessagingSection({ data }: { data: PrivacyOverview }) {
  const { t } = useI18n()
  const counts = data.messaging.ok ? data.messaging.data : null
  const rows = [
    {
      icon: <Inbox size={16} />,
      who: t('settings.privacy.msg.friends', 'Zone members and your contacts'),
      where: t('settings.privacy.msg.inbox', 'Delivered to your inbox'),
      count: null,
    },
    {
      icon: <MessageCircle size={16} />,
      who: t('settings.privacy.msg.strangers', 'People not in your contacts'),
      where: t('settings.privacy.msg.requests', 'Held as message requests until you accept'),
      count: counts ? t('settings.privacy.msg.strangerCount', '{{count}} strangers on record', { count: counts.strangers }) : null,
    },
    {
      icon: <ShieldBan size={16} />,
      who: t('settings.privacy.msg.blocked', 'Blocked contacts'),
      where: t('settings.privacy.msg.dropped', 'Messages are dropped'),
      count: counts ? t('settings.privacy.msg.blockedCount', '{{count}} blocked', { count: counts.blocked }) : null,
    },
  ]

  return (
    <Section title={t('settings.privacy.messagingAccess', 'Messaging Access')} testId="settings-privacy-messaging">
      <div className="space-y-2">
        {rows.map((row) => (
          <ListCard key={row.who}>
            <div className="flex items-start gap-3">
              <span className="mt-0.5 shrink-0" style={{ color: 'var(--cp-muted)' }}>{row.icon}</span>
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{row.who}</p>
                <p className="mt-0.5 text-xs" style={{ color: 'var(--cp-muted)' }}>{row.where}</p>
              </div>
              {row.count ? <span className="shrink-0 text-xs" style={{ color: 'var(--cp-muted)' }}>{row.count}</span> : null}
            </div>
          </ListCard>
        ))}
        {!data.messaging.ok ? (
          <SectionNote tone="danger">{t('settings.privacy.sourceUnavailable', 'Could not be read: {{detail}}', { detail: data.messaging.error })}</SectionNote>
        ) : null}
        <SectionNote>{t('settings.privacy.messagingFixed', 'These rules are built into Message Center and cannot be changed yet. Manage contacts and requests in MessageHub.')}</SectionNote>
      </div>
    </Section>
  )
}

function DataVisibilitySection({ userId }: { userId: string }) {
  const { t } = useI18n()
  const areas = [
    {
      path: `/home/${userId}/`,
      icon: Lock,
      scope: <StatusBadge status="pass" label={t('settings.privacy.scope.personal', 'Personal')} />,
      title: t('settings.privacy.area.home', 'Your files'),
      detail: t('settings.privacy.area.homeDesc', 'Your personal folder. Apps cannot reach it unless you grant them a folder when installing.'),
    },
    {
      path: `/home/${userId}/.local/share/<app>/`,
      icon: FolderLock,
      scope: <StatusBadge status="pass" label={t('settings.privacy.scope.app', 'App only')} />,
      title: t('settings.privacy.area.appData', 'App data'),
      detail: t('settings.privacy.area.appDataDesc', 'Each app keeps its data here and only that app reads and writes it. You can delete it when uninstalling the app.'),
    },
    {
      path: `/home/${userId}/shared/`,
      icon: Users,
      scope: <StatusBadge status="info" label={t('settings.privacy.scope.shared', 'Shared')} />,
      title: t('settings.privacy.area.shared', 'Your shared folder'),
      detail: t('settings.privacy.area.sharedDesc', 'What you share with other users and apps; normally read-only for them.'),
    },
    {
      path: '/srv/library/',
      icon: Users,
      scope: <StatusBadge status="info" label={t('settings.privacy.scope.zone', 'Zone')} />,
      title: t('settings.privacy.area.library', 'Zone library'),
      detail: t('settings.privacy.area.libraryDesc', 'Shared by every user of this Zone: anyone can read and add files, and delete only their own.'),
    },
    {
      path: '/srv/publish/',
      icon: Globe,
      scope: <StatusBadge status="info" label={t('settings.privacy.scope.zone', 'Zone')} />,
      title: t('settings.privacy.area.publish', 'Zone published data'),
      detail: t('settings.privacy.area.publishDesc', 'Data the Zone shares out; normally read-only.'),
    },
  ]

  return (
    <Section title={t('settings.privacy.dataVisibility', 'Data Visibility')} testId="settings-privacy-data">
      <div className="space-y-2">
        {areas.map((area) => (
          <ListCard key={area.path}>
            <div className="flex items-start gap-3">
              <area.icon size={16} className="mt-0.5 shrink-0" style={{ color: 'var(--cp-muted)' }} />
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{area.title}</span>
                  {area.scope}
                </div>
                <p className="mt-1 break-all font-mono text-xs" style={{ color: 'var(--cp-muted)' }}>{area.path}</p>
                <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>{area.detail}</p>
              </div>
            </div>
          </ListCard>
        ))}
      </div>
    </Section>
  )
}

function AppRow({ app, userId }: { app: AppAccessEntry; userId: string }) {
  const { t, locale } = useI18n()
  const detail = app.detail

  return (
    <ListCard testId={`settings-privacy-app-${app.appInstanceId}`}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          {app.iconUrl ? (
            <img src={app.iconUrl} alt="" className="h-5 w-5 shrink-0 rounded" />
          ) : (
            <LayoutGrid size={16} className="shrink-0" style={{ color: 'var(--cp-muted)' }} />
          )}
          <span className="truncate text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{pickLocalized(app.name, locale) ?? app.appInstanceId}</span>
        </div>
        {detail?.ok ? audienceBadge(detail.data.audience, detail.data.sharedUsers, t) : null}
      </div>
      {app.ownerUserId !== userId ? (
        <p className="mt-1 text-xs" style={{ color: 'var(--cp-muted)' }}>{t('settings.privacy.ownedBy', 'Installed by {{owner}}', { owner: app.ownerUserId })}</p>
      ) : null}
      {detail && !detail.ok ? (
        <p className="mt-1 text-xs" style={{ color: 'var(--cp-danger)' }}>{t('settings.privacy.sourceUnavailable', 'Could not be read: {{detail}}', { detail: detail.error })}</p>
      ) : null}
      {detail?.ok ? (
        <div className="mt-2 space-y-1.5 text-xs" style={{ color: 'var(--cp-muted)' }}>
          {detail.data.mounts.length > 0 ? (
            <div>
              <p>{t('settings.privacy.folderAccess', 'Folder access')}</p>
              <ul className="mt-1 space-y-0.5">
                {detail.data.mounts.map((mount) => (
                  <li key={`${mount.kind}:${mount.path}`} className="flex flex-wrap gap-x-2">
                    <span className="break-all font-mono" style={{ color: 'var(--cp-text)' }}>{mountTarget(mount)}</span>
                    <span>{accessLabel(mount.access, t)}</span>
                  </li>
                ))}
              </ul>
            </div>
          ) : (
            <p>{t('settings.privacy.ownDataOnly', 'Folder access: only its own data folder.')}</p>
          )}
          {detail.data.permissions.length > 0 ? (
            <div className="flex flex-wrap gap-1">
              {detail.data.permissions.map((permission) => (
                <span
                  key={permission.scope}
                  className="rounded px-1.5 py-0.5"
                  style={{ color: 'var(--cp-text)', background: 'color-mix(in srgb, var(--cp-surface) 60%, transparent)' }}
                  title={permission.scope}
                >
                  {permissionLabel(permission.scope, t)}
                  {permission.actions.length > 0 ? ` · ${permission.actions.join('/')}` : ''}
                </span>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}
    </ListCard>
  )
}

function AppAccessSection({ data }: { data: PrivacyOverview }) {
  const { t } = useI18n()

  return (
    <Section title={t('settings.privacy.appAgentAccess', 'App & Agent Access')} testId="settings-privacy-apps">
      <div className="space-y-2">
        <SectionState value={data.apps}>
          {(apps) =>
            apps.length > 0 ? (
              apps.map((app) => <AppRow key={app.appInstanceId} app={app} userId={data.userId} />)
            ) : (
              <SectionNote>{t('settings.privacy.noApps', 'No app is installed for you.')}</SectionNote>
            )
          }
        </SectionState>
        <SectionState value={data.agents}>
          {(agents) =>
            agents.map((agent) => (
              <ListCard key={agent.agentId} testId={`settings-privacy-agent-${agent.agentId}`}>
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="flex min-w-0 items-center gap-2">
                    <Bot size={16} className="shrink-0" style={{ color: 'var(--cp-muted)' }} />
                    <span className="truncate text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{agent.name}</span>
                    <StatusBadge status="neutral" label={t('settings.privacy.agent', 'Agent')} />
                  </div>
                  <StatusBadge
                    status={agent.enabled ? 'pass' : 'neutral'}
                    label={agent.enabled ? t('settings.privacy.agentEnabled', 'Enabled') : t('settings.privacy.agentDisabled', 'Disabled')}
                  />
                </div>
                <ul className="mt-2 space-y-0.5 text-xs" style={{ color: 'var(--cp-muted)' }}>
                  {agent.ownerUserId !== data.userId ? <li>{t('settings.privacy.agentOwner', 'Owned by {{owner}}', { owner: agent.ownerUserId })}</li> : null}
                  <li>
                    {agent.allowGroup
                      ? t('settings.privacy.agentGroups', 'Can join group chats and read their messages')
                      : t('settings.privacy.agentNoGroups', 'Does not join group chats')}
                  </li>
                  {agent.tunnels.length > 0 ? (
                    <li>{t('settings.privacy.agentTunnels', 'Connected to {{platforms}}: messages there reach this Agent', { platforms: agent.tunnels.join(', ') })}</li>
                  ) : null}
                  <li>{t('settings.privacy.agentData', 'Keeps its data in the folder of its app')}{agent.runtimeApp ? ` (${agent.runtimeApp})` : ''}</li>
                </ul>
              </ListCard>
            ))
          }
        </SectionState>
      </div>
    </Section>
  )
}

export function PrivacyPage() {
  const { t } = useI18n()
  const store = useSettingsStore()
  const privacy = useSettingsResource('privacy')
  const data = privacy.data

  const refresh = (
    <Tooltip title={t('settings.common.refresh', 'Refresh')}>
      <span>
        <IconButton size="small" aria-label={t('settings.common.refresh', 'Refresh')} disabled={privacy.status === 'loading'} onClick={() => void store.load('privacy', true)}>
          <RefreshCw size={14} />
        </IconButton>
      </span>
    </Tooltip>
  )

  return (
    <div className="space-y-4" data-testid="settings-privacy-page">
      {data ? (
        <>
          <PublicAccessSection data={data} actions={refresh} />
          <MessagingSection data={data} />
          <DataVisibilitySection userId={data.userId} />
          <AppAccessSection data={data} />
        </>
      ) : privacy.status === 'error' ? (
        <SectionNote tone="danger">{t('settings.privacy.unavailable', 'Privacy information could not be loaded: {{detail}}', { detail: privacy.error ?? '' })}</SectionNote>
      ) : (
        <SectionNote>{t('settings.common.loading', 'Loading…')}</SectionNote>
      )}
    </div>
  )
}
