import { useMemo } from 'react'

import Icon from '../icons'

const users: UserSummary[] = [
  { name: 'Alice Johnson', email: 'alice@buckyos.io', role: 'Owner', status: 'active', avatar: 'https://i.pravatar.cc/64?img=32' },
  { name: 'Leo Martins', email: 'leo@buckyos.io', role: 'Admin', status: 'active', avatar: 'https://i.pravatar.cc/64?img=15' },
  { name: 'Rina Patel', email: 'rina@buckyos.io', role: 'Editor', status: 'pending', avatar: 'https://i.pravatar.cc/64?img=47' },
  { name: 'Tomas Silva', email: 'tomas@buckyos.io', role: 'Viewer', status: 'disabled', avatar: 'https://i.pravatar.cc/64?img=24' },
  { name: 'Mina Cho', email: 'mina@buckyos.io', role: 'Editor', status: 'active', avatar: 'https://i.pravatar.cc/64?img=18' },
]

const UserManagementPage = () => {
  const stats = useMemo(
    () => [
      { label: 'Total Users', value: users.length, tone: 'sky' },
      { label: 'Pending Invites', value: users.filter((u) => u.status === 'pending').length, tone: 'amber' },
      { label: 'Disabled', value: users.filter((u) => u.status === 'disabled').length, tone: 'rose' },
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

      <section className="grid gap-4 sm:grid-cols-3">
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
        <div className="mb-4 flex items-center justify-between text-[var(--cp-ink)]">
          <div className="flex items-center gap-3 text-lg font-semibold">
            <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="users" className="size-4" />
            </span>
            <span>Team Members</span>
          </div>
          <div className="text-xs text-[var(--cp-muted)]">Roles: Owner / Admin / Editor / Viewer</div>
        </div>
        <div className="divide-y divide-[var(--cp-border)]">
          {users.map((user) => (
            <div key={user.email} className="grid grid-cols-[auto_1fr_auto_auto] items-center gap-4 py-3">
              <img
                src={user.avatar}
                alt={`${user.name} avatar`}
                className="size-10 rounded-full border border-[var(--cp-border)] object-cover"
              />
              <div className="leading-tight">
                <p className="font-medium text-[var(--cp-ink)]">{user.name}</p>
                <p className="text-xs text-[var(--cp-muted)]">{user.email}</p>
              </div>
              <span className="rounded-full border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-1 text-xs text-[var(--cp-ink)]">
                {user.role}
              </span>
              <span className={`rounded-full px-3 py-1 text-xs ${badgeClass[user.status]}`}>{user.status}</span>
            </div>
          ))}
        </div>
      </section>
    </div>
  )
}

export default UserManagementPage
