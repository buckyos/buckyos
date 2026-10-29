import { lazy, Suspense } from 'react'
import { Navigate, RouterProvider, createBrowserRouter } from 'react-router-dom'
import { DesktopBackground } from './desktop/DesktopBackground'
import {
  DesktopBackgroundProvider,
  useDesktopBackground,
} from './desktop/DesktopBackgroundProvider'
import { I18nProvider } from './i18n/provider'
import { PrototypeThemeProvider } from './theme/provider'
import { DesktopRoute } from './desktop/DesktopRoute'
import { isPortalRoot } from './app/homestation/links'
import { useDocumentBrand } from './presentation/documentHead'
import { useZonePresentation } from './presentation/zone'
import { getBootIdentity } from './sso/identity'
import type { CurrentIdentity } from 'buckyos'

// Standalone routes are code-split like the app panels: a visitor of `/`
// should not download MessageHub / HomeStation / TaskCenter / the installer
// up front, and a visitor of `/messagehub` should not download the desktop.
const LoginPage = lazy(() => import('./auth/LoginPage'))
const HomeStationRoute = lazy(() =>
  import('./app/homestation/HomeStationRoute').then((m) => ({ default: m.HomeStationRoute })),
)
const HomeStationPortalRoute = lazy(() =>
  import('./app/homestation/HomeStationPortalRoute').then((m) => ({ default: m.HomeStationPortalRoute })),
)

const portalRoute = (
  <Suspense fallback={null}>
    <HomeStationPortalRoute />
  </Suspense>
)
const MessageHubRoute = lazy(() =>
  import('./app/messagehub/MessageHubRoute').then((m) => ({ default: m.MessageHubRoute })),
)
const MessageHubGuestRoute = lazy(() =>
  import('./app/messagehub/guest/GuestRoute').then((m) => ({ default: m.MessageHubGuestRoute })),
)
const AIWorkspaceRoute = lazy(() =>
  import('./app/aiworkspace/AIWorkspaceRoute').then((m) => ({ default: m.AIWorkspaceRoute })),
)
const PrompterRoute = lazy(() =>
  import('./app/aiworkspace/presentation/PrompterRoute').then((m) => ({ default: m.PrompterRoute })),
)
const TaskCenterRoute = lazy(() =>
  import('./app/task-center/TaskCenterRoute').then((m) => ({ default: m.TaskCenterRoute })),
)
const UserProfileRoute = lazy(() =>
  import('./userprofile').then((m) => ({ default: m.UserProfileRoute })),
)
const AppInstallerRoute = lazy(() =>
  import('./sysdlg').then((m) => ({ default: m.AppInstallerRoute })),
)
const RestrictedHome = lazy(() =>
  import('./sso/RestrictedHome').then((m) => ({ default: m.RestrictedHome })),
)
const SsoEntryPage = lazy(() =>
  import('./sso/EntryPage').then((m) => ({ default: m.EntryPage })),
)
const SsoInvitePage = lazy(() =>
  import('./sso/InvitePage').then((m) => ({ default: m.InvitePage })),
)
const SsoFederatePage = lazy(() =>
  import('./sso/FederatePage').then((m) => ({ default: m.FederatePage })),
)
const UserInvitePage = lazy(() =>
  import('./sso/UserInvitePage').then((m) => ({ default: m.UserInvitePage })),
)

/**
 * The restricted shell at `/` is for External DIDs (no account here) and for sessions an exclusive
 * invite link opened. A Limited account signed in with its password keeps the Desktop: its own
 * settings, profile, address book and allowed Apps stay reachable, and what its session may not
 * use answers with a `session_kind_not_allowed` denial.
 */
function usesRestrictedShell(identity: CurrentIdentity): boolean {
  return identity.kind === 'external'
    || (identity.kind === 'account' && identity.restricted && identity.auth_method === 'exclusive_invite')
}

function RootRoute() {
  return usesRestrictedShell(getBootIdentity()) ? (
    <Suspense fallback={null}>
      <RestrictedHome />
    </Suspense>
  ) : (
    <DesktopRoute />
  )
}

const router = createBrowserRouter([
  {
    path: '/',
    // `www.<zone>/` and `homestation.<zone>/` open the zone's default HomeStation feed.
    element: isPortalRoot() ? portalRoute : <RootRoute />,
  },
  {
    path: '/login',
    element: (
      <Suspense fallback={null}>
        <LoginPage />
      </Suspense>
    ),
  },
  {
    path: '/homestation',
    element: (
      <Suspense fallback={null}>
        <HomeStationRoute />
      </Suspense>
    ),
  },
  // A user's home feed (`~zone`: the zone feed) and one of its posts: public, login optional.
  { path: '/homestation/:feed', element: portalRoute },
  { path: '/homestation/:feed/:key', element: portalRoute },
  {
    path: '/messagehub',
    element: (
      <Suspense fallback={null}>
        <MessageHubRoute />
      </Suspense>
    ),
  },
  {
    // a Session Only visitor's group Sessions (SSO-12): login optional, signs in through the group's entry
    path: '/messagehub/guest',
    element: (
      <Suspense fallback={null}>
        <MessageHubGuestRoute />
      </Suspense>
    ),
  },
  {
    // the signed-in user's workspace list; a workspace itself lives on its own origin (AI Workspace T4-8)
    path: '/workspace',
    element: (
      <Suspense fallback={null}>
        <AIWorkspaceRoute />
      </Suspense>
    ),
  },
  {
    // the prompter link of a show (AI Workspace 第三期规划 §11): works without signing in
    path: '/workspace/:workspaceId/show/:showId',
    element: (
      <Suspense fallback={null}>
        <PrompterRoute />
      </Suspense>
    ),
  },
  {
    path: '/taskcenter',
    element: (
      <Suspense fallback={null}>
        <TaskCenterRoute />
      </Suspense>
    ),
  },
  {
    path: '/userprofile',
    element: (
      <Suspense fallback={null}>
        <UserProfileRoute />
      </Suspense>
    ),
  },
  {
    path: '/sso/entry',
    element: (
      <Suspense fallback={null}>
        <SsoEntryPage />
      </Suspense>
    ),
  },
  {
    path: '/sso/invite/:inviteId',
    element: (
      <Suspense fallback={null}>
        <SsoInvitePage />
      </Suspense>
    ),
  },
  {
    path: '/users/invite/:inviteId',
    element: (
      <Suspense fallback={null}>
        <UserInvitePage />
      </Suspense>
    ),
  },
  {
    path: '/sso/federate',
    element: (
      <Suspense fallback={null}>
        <SsoFederatePage />
      </Suspense>
    ),
  },
  {
    path: '/sysdlg/app_installer',
    element: (
      <Suspense fallback={null}>
        <AppInstallerRoute />
      </Suspense>
    ),
  },
  {
    path: '*',
    element: <Navigate to="/" replace />,
  },
])

/** Every page shows the Zone's name and icon in the browser tab. */
function ZoneBrand() {
  const zone = useZonePresentation()
  useDocumentBrand(zone.status === 'loading' ? undefined : zone.title, zone.icons[0])
  return null
}

function AppShell() {
  const { background } = useDesktopBackground()

  return (
    <>
      <ZoneBrand />
      <DesktopBackground
        wallpaper={background.wallpaper}
        pageCount={background.pageCount}
      />
      <RouterProvider router={router} />
    </>
  )
}

function App() {
  return (
    <PrototypeThemeProvider>
      <I18nProvider>
        <DesktopBackgroundProvider>
          <AppShell />
        </DesktopBackgroundProvider>
      </I18nProvider>
    </PrototypeThemeProvider>
  )
}

export default App
