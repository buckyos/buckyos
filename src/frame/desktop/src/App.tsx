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

const router = createBrowserRouter([
  {
    path: '/',
    // `www.<zone>/` and `homestation.<zone>/` open the zone's default HomeStation feed.
    element: isPortalRoot() ? portalRoute : <DesktopRoute />,
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
    path: '/workspace/:workspaceId?',
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

function AppShell() {
  const { background } = useDesktopBackground()

  return (
    <>
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
