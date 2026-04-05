import { Navigate, RouterProvider, createBrowserRouter } from 'react-router-dom'
import { DesktopBackground } from './desktop/DesktopBackground'
import {
  DesktopBackgroundProvider,
  useDesktopBackground,
} from './desktop/DesktopBackgroundProvider'
import { I18nProvider } from './i18n/provider'
import { PrototypeThemeProvider } from './theme/provider'
import { DesktopRoute } from './desktop/DesktopRoute'
import { FilesRoute } from './app/files/FilesRoute'
import { HomeStationRoute } from './app/homestation/HomeStationRoute'
import { MessageHubRoute } from './app/messagehub/MessageHubRoute'
import { TaskCenterRoute } from './app/task-center/TaskCenterRoute'

const router = createBrowserRouter([
  {
    path: '/',
    element: <DesktopRoute />,
  },
  {
    path: '/files',
    element: <FilesRoute />,
  },
  {
    path: '/homestation',
    element: <HomeStationRoute />,
  },
  {
    path: '/messagehub',
    element: <MessageHubRoute />,
  },
  {
    path: '/taskcenter',
    element: <TaskCenterRoute />,
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
        viewportProgress={background.viewportProgress}
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
