import { useMemo } from 'react'

const users: UserSummary[] = [
  { name: 'Alice Johnson', email: 'alice@buckyos.io', role: 'Owner', status: 'active', avatar: 'https://i.pravatar.cc/64?img=32' },
  { name: 'Leo Martins', email: 'leo@buckyos.io', role: 'Admin', status: 'active', avatar: 'https://i.pravatar.cc/64?img=15' },
  { name: 'Rina Patel', email: 'rina@buckyos.io', role: 'Editor', status: 'pending', avatar: 'https://i.pravatar.cc/64?img=47' },
  { name: 'Tomás Silva', email: 'tomas@buckyos.io', role: 'Viewer', status: 'disabled', avatar: 'https://i.pravatar.cc/64?img=24' },
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
    active: 'bg-emerald-500/15 text-emerald-300',
    pending: 'bg-amber-500/15 text-amber-200',
    disabled: 'bg-rose-500/15 text-rose-200',
  }

  return (
    <div className="space-y-6">
      <header className="flex flex-wrap items-center justify-between gap-4 rounded-3xl border border-slate-900/60 bg-slate-900/50 px-8 py-6 shadow-lg shadow-black/20 backdrop-blur">
        <div>
          <h1 className="text-2xl font-semibold text-white sm:text-3xl">User Management</h1>
          <p className="text-sm text-slate-400">
            Control team access, roles, and invitations for your control panel.
          </p>
        </div>
        <button
          type="button"
          className="inline-flex items-center gap-2 rounded-full bg-sky-500 px-4 py-2 text-sm font-medium text-white shadow hover:bg-sky-400"
        >
          + Invite User
        </button>
      </header>

      <section className="grid gap-4 sm:grid-cols-3">
        {stats.map((item) => (
          <div
            key={item.label}
            className="rounded-2xl border border-slate-900/80 bg-slate-900/60 px-4 py-4 text-sm text-slate-300 shadow"
          >
            <p className="text-xs uppercase tracking-wide text-slate-500">{item.label}</p>
            <p className="mt-1 text-3xl font-semibold text-white">{item.value}</p>
          </div>
        ))}
      </section>

      <section className="rounded-3xl border border-slate-900/60 bg-slate-900/60 p-6 shadow-lg shadow-black/20">
        <div className="mb-4 flex items-center justify-between text-white">
          <div className="flex items-center gap-2 text-lg font-semibold">
            <span aria-hidden>👥</span>
            <span>Team Members</span>
          </div>
          <div className="text-xs text-slate-400">Roles: Owner • Admin • Editor • Viewer</div>
        </div>
        <div className="divide-y divide-slate-800">
          {users.map((user) => (
            <div key={user.email} className="grid grid-cols-[auto_1fr_auto_auto] items-center gap-4 py-3">
              <img
                src={user.avatar}
                alt={`${user.name} avatar`}
                className="size-10 rounded-full border border-slate-800 object-cover"
              />
              <div className="leading-tight">
                <p className="font-medium text-white">{user.name}</p>
                <p className="text-xs text-slate-400">{user.email}</p>
              </div>
              <span className="rounded-full bg-slate-800 px-3 py-1 text-xs text-slate-200">{user.role}</span>
              <span className={`rounded-full px-3 py-1 text-xs ${badgeClass[user.status]}`}>{user.status}</span>
            </div>
          ))}
        </div>
      </section>
    </div>
  )
}

export default UserManagementPage
