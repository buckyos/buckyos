import { useMemo } from 'react'

import Icon from '../icons'

type TeamMember = UserSummary & {
  group: string
  mfa: 'required' | 'optional'
  zone: string
  lastSeen: string
}

const teamMembers: TeamMember[] = [
  {
    name: 'Alice Johnson',
    email: 'alice@buckyos.io',
    role: 'Owner',
    status: 'active',
    avatar: 'https://i.pravatar.cc/64?img=32',
    group: 'Administrators',
    mfa: 'required',
    zone: 'meteor101',
    lastSeen: 'Online now',
  },
  {
    name: 'Leo Martins',
    email: 'leo@buckyos.io',
    role: 'Admin',
    status: 'active',
    avatar: 'https://i.pravatar.cc/64?img=15',
    group: 'Administrators',
    mfa: 'required',
    zone: 'meteor101',
    lastSeen: '2 min ago',
  },
  {
    name: 'Rina Patel',
    email: 'rina@buckyos.io',
    role: 'Editor',
    status: 'pending',
    avatar: 'https://i.pravatar.cc/64?img=47',
    group: 'Product',
    mfa: 'optional',
    zone: 'staging',
    lastSeen: 'Invite pending',
  },
  {
    name: 'Tomas Silva',
    email: 'tomas@buckyos.io',
    role: 'Viewer',
    status: 'disabled',
    avatar: 'https://i.pravatar.cc/64?img=24',
    group: 'Guests',
    mfa: 'optional',
    zone: 'sandbox',
    lastSeen: '14 days ago',
  },
  {
    name: 'Mina Cho',
    email: 'mina@buckyos.io',
    role: 'Editor',
    status: 'active',
    avatar: 'https://i.pravatar.cc/64?img=18',
    group: 'Product',
    mfa: 'required',
    zone: 'meteor101',
    lastSeen: '35 min ago',
  },
]

const userGroups = [
  { name: 'Administrators', members: 2, policy: 'Full control + policy edits' },
  { name: 'Product', members: 2, policy: 'Read/Write configs, no destructive ops' },
  { name: 'Guests', members: 1, policy: 'Read-only dashboard + logs' },
]

const pendingInvites = [
  { email: 'nora@buckyos.io', role: 'Editor', expires: 'in 3 days' },
  { email: 'ops-oncall@buckyos.io', role: 'Admin', expires: 'in 6 days' },
]

const accessPolicies = [
  { key: 'MFA Policy', value: 'Required for Owner/Admin roles', tone: 'ready' as const },
  { key: 'Session Timeout', value: '12h idle timeout', tone: 'ready' as const },
  { key: 'Invite Approval', value: 'Admin review required', tone: 'review' as const },
  { key: 'Break-glass Access', value: 'Emergency account configured', tone: 'ready' as const },
]

const recentEvents = [
  { title: 'Admin role granted to leo@buckyos.io', time: '8 min ago' },
  { title: 'Invite sent to nora@buckyos.io', time: '1 hour ago' },
  { title: 'Guest account disabled for tomas@buckyos.io', time: 'Yesterday' },
]

const UserManagementPage = () => {
  const stats = useMemo(
    () => [
      { label: 'Total Users', value: teamMembers.length },
      { label: 'Pending Invites', value: pendingInvites.length },
      { label: 'Disabled Accounts', value: teamMembers.filter((u) => u.status === 'disabled').length },
      { label: 'MFA Coverage', value: `${Math.round((teamMembers.filter((u) => u.mfa === 'required').length / teamMembers.length) * 100)}%` },
    ],
    [],
  )

  const badgeClass: Record<UserSummary['status'], string> = {
    active: 'bg-emerald-100 text-emerald-700',
    pending: 'bg-amber-100 text-amber-700',
    disabled: 'bg-rose-100 text-rose-700',
  }

  return (
    <div className="space-y-6">
      <header className="cp-panel flex flex-wrap items-center justify-between gap-4 px-8 py-6">
        <div>
          <h1 className="text-2xl font-semibold text-[var(--cp-ink)] sm:text-3xl">
            User Management
          </h1>
          <p className="text-sm text-[var(--cp-muted)]">
            Control team access, roles, and invitations for your control panel.
          </p>
        </div>
        <button
          type="button"
          className="inline-flex items-center gap-2 rounded-full bg-[var(--cp-primary)] px-4 py-2 text-sm font-medium text-white shadow transition hover:bg-[var(--cp-primary-strong)]"
        >
          Invite User
        </button>
      </header>

      <section className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {stats.map((item) => (
          <div
            key={item.label}
            className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface)] px-4 py-4 text-sm text-[var(--cp-muted)] shadow-sm"
          >
            <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">{item.label}</p>
            <p className="mt-1 text-3xl font-semibold text-[var(--cp-ink)]">{item.value}</p>
          </div>
        ))}
      </section>

      <section className="cp-panel p-6">
        <div className="mb-4 flex items-center justify-between gap-3 text-[var(--cp-ink)]">
          <div className="flex items-center gap-3 text-lg font-semibold">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="users" className="size-4" />
            </span>
            <span>Team Members</span>
          </div>
          <div className="text-xs text-[var(--cp-muted)]">Roles: Owner / Admin / Editor / Viewer</div>
        </div>
        <div className="divide-y divide-[var(--cp-border)]">
          {teamMembers.map((user) => (
            <div key={user.email} className="grid grid-cols-[auto_1fr_auto_auto] items-center gap-4 py-3">
              <img
                src={user.avatar}
                alt={`${user.name} avatar`}
                className="size-10 rounded-full border border-[var(--cp-border)] object-cover"
              />
              <div className="leading-tight">
                <p className="font-medium text-[var(--cp-ink)]">{user.name}</p>
                <p className="text-xs text-[var(--cp-muted)]">{user.email}</p>
                <p className="text-[11px] text-[var(--cp-muted)]">
                  {user.group} - {user.zone} - {user.lastSeen}
                </p>
              </div>
              <div className="flex flex-wrap items-center justify-end gap-2">
                <span className="rounded-full border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-1 text-xs text-[var(--cp-ink)]">
                  {user.role}
                </span>
                <span
                  className={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${
                    user.mfa === 'required' ? 'bg-teal-100 text-teal-700' : 'bg-slate-100 text-slate-700'
                  }`}
                >
                  MFA {user.mfa}
                </span>
              </div>
              <span className={`rounded-full px-3 py-1 text-xs ${badgeClass[user.status]}`}>{user.status}</span>
            </div>
          ))}
        </div>
      </section>

      <section className="grid gap-6 lg:grid-cols-2">
        <div className="cp-panel p-6">
          <div className="mb-4 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="shield" className="size-4" />
            </span>
            <h2>Groups and Permissions</h2>
          </div>
          <div className="space-y-2">
            {userGroups.map((group) => (
              <div
                key={group.name}
                className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2"
              >
                <div className="flex items-center justify-between gap-3">
                  <p className="text-sm font-semibold text-[var(--cp-ink)]">{group.name}</p>
                  <span className="rounded-full border border-[var(--cp-border)] bg-white px-2.5 py-0.5 text-[11px] text-[var(--cp-ink)]">
                    {group.members} members
                  </span>
                </div>
                <p className="mt-1 text-xs text-[var(--cp-muted)]">{group.policy}</p>
              </div>
            ))}
          </div>
        </div>

        <div className="cp-panel p-6">
          <div className="mb-4 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="bell" className="size-4" />
            </span>
            <h2>Pending Invites</h2>
          </div>
          <div className="space-y-2">
            {pendingInvites.map((invite) => (
              <div
                key={invite.email}
                className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2"
              >
                <div className="flex items-center justify-between gap-3">
                  <p className="text-sm font-semibold text-[var(--cp-ink)]">{invite.email}</p>
                  <span className="rounded-full bg-amber-100 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-amber-700">
                    {invite.expires}
                  </span>
                </div>
                <p className="mt-1 text-xs text-[var(--cp-muted)]">Role: {invite.role}</p>
              </div>
            ))}
          </div>
        </div>
      </section>

      <section className="grid gap-6 lg:grid-cols-2">
        <div className="cp-panel p-6">
          <div className="mb-4 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="settings" className="size-4" />
            </span>
            <h2>Access Policies</h2>
          </div>
          <div className="space-y-2">
            {accessPolicies.map((policy) => (
              <div
                key={policy.key}
                className="flex items-center justify-between gap-3 rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2"
              >
                <div>
                  <p className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">{policy.key}</p>
                  <p className="text-xs text-[var(--cp-ink)]">{policy.value}</p>
                </div>
                <span
                  className={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${
                    policy.tone === 'ready'
                      ? 'bg-emerald-100 text-emerald-700'
                      : 'bg-amber-100 text-amber-700'
                  }`}
                >
                  {policy.tone}
                </span>
              </div>
            ))}
          </div>
        </div>

        <div className="cp-panel p-6">
          <div className="mb-4 flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="chart" className="size-4" />
            </span>
            <h2>Recent Access Events</h2>
          </div>
          <div className="space-y-2">
            {recentEvents.map((event) => (
              <div
                key={`${event.title}-${event.time}`}
                className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-2"
              >
                <p className="text-sm font-semibold text-[var(--cp-ink)]">{event.title}</p>
                <p className="text-xs text-[var(--cp-muted)]">{event.time}</p>
              </div>
            ))}
          </div>
        </div>
      </section>
    </div>
  )
}

export default UserManagementPage
