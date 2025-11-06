import { NavLink, Outlet } from 'react-router-dom'

type NavItem = {
  label: string
  icon: string
  path: string
  badge?: string
}

const primaryNav: NavItem[] = [
  { label: 'Dashboard', icon: '📊', path: '/' },
  { label: 'User Management', icon: '👥', path: '/users' },
  { label: 'Storage', icon: '🗄️', path: '/storage' },
  { label: 'dApp Store', icon: '🛒', path: '/dapps' },
  { label: 'Settings', icon: '⚙️', path: '/settings' },
]

const secondaryNav: NavItem[] = [
  { label: 'Notifications', icon: '🔔', path: '/notifications', badge: '3' },
  { label: 'Sign Out', icon: '↪️', path: '/sign-out' },
]

const baseNavClasses =
  'flex w-full items-center gap-3 rounded-xl px-3.5 py-2.5 text-left transition'

const RootLayout = () => {
  return (
    <div className="min-h-screen bg-slate-950 text-slate-100">
      <div className="mx-auto flex h-screen max-w-7xl gap-8 overflow-hidden px-6 py-10 lg:px-10">
        <aside className="flex h-full w-64 flex-col rounded-3xl bg-slate-900/60 p-6 backdrop-blur">
          <div className="mb-10 flex items-center gap-3 text-lg font-semibold tracking-tight">
            <span className="inline-flex size-10 items-center justify-center rounded-2xl bg-sky-500 text-2xl">
              B
            </span>
            <div className="flex flex-col leading-tight">
              <span>BuckyOS</span>
              <span className="text-xs font-normal text-slate-400">Control Panel</span>
            </div>
          </div>

          <nav className="space-y-8 text-sm">
            <div className="space-y-1.5">
              {primaryNav.map((item) => (
                <NavLink
                  key={item.path}
                  to={item.path}
                  end={item.path === '/'}
                  className={({ isActive }) =>
                    [
                      baseNavClasses,
                      isActive
                        ? 'bg-sky-500 text-white shadow-lg shadow-sky-500/30'
                        : 'text-slate-300 hover:bg-slate-800/80 hover:text-white',
                    ].join(' ')
                  }
                >
                  <span aria-hidden>{item.icon}</span>
                  <span className="flex-1">{item.label}</span>
                </NavLink>
              ))}
            </div>

            <div className="space-y-1.5">
              {secondaryNav.map((item) => (
                <NavLink
                  key={item.path}
                  to={item.path}
                  className={({ isActive }) =>
                    [
                      baseNavClasses,
                      isActive
                        ? 'bg-slate-800/90 text-white'
                        : 'text-slate-300 hover:bg-slate-800/80 hover:text-white',
                    ].join(' ')
                  }
                >
                  <span aria-hidden>{item.icon}</span>
                  <span className="flex-1">{item.label}</span>
                  {item.badge ? (
                    <span className="inline-flex min-w-6 items-center justify-center rounded-full bg-rose-500 px-1.5 text-xs font-semibold text-white">
                      {item.badge}
                    </span>
                  ) : null}
                </NavLink>
              ))}
            </div>
          </nav>

          <div className="mt-auto space-y-4 rounded-2xl bg-slate-900/80 p-4 text-sm text-slate-300">
            <div className="flex items-center gap-3">
              <img
                src="https://i.pravatar.cc/64?img=12"
                alt="Admin user avatar"
                className="size-10 rounded-full border border-slate-700 object-cover"
              />
              <div className="leading-tight">
                <p className="font-medium text-white">Admin User</p>
                <p className="text-xs text-slate-400">admin@buckyos.io</p>
              </div>
            </div>
            <div className="rounded-xl border border-slate-800 bg-slate-900/60 px-4 py-3 text-xs leading-5 text-slate-400">
              <div className="mb-1 flex items-center gap-2 text-slate-200">
                <span className="size-2 rounded-full bg-emerald-400" aria-hidden />
                System Online
              </div>
              <div className="flex justify-between">
                <span>Network</span>
                <span className="text-white">847 peers</span>
              </div>
              <div className="flex justify-between">
                <span>Active Sessions</span>
                <span className="text-white">23</span>
              </div>
            </div>
          </div>
        </aside>

        <main className="flex-1 overflow-y-auto pb-10 pr-2">
          <Outlet />
        </main>
      </div>
    </div>
  )
}

export default RootLayout
