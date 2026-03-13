import { Navigate, createBrowserRouter } from 'react-router-dom'
import RootLayout from '../ui/RootLayout'
import DashboardPage from '../ui/pages/DashboardPage'
import DesktopHomePage from '../ui/pages/DesktopHomePage'
import LoginPage from '../ui/pages/LoginPage'
import SsoLoginPage from '../ui/pages/SsoLoginPage'
import NotFoundPage from '../ui/pages/NotFoundPage'
import RequireAuth from '../ui/auth/RequireAuth'
import FileManagerPage from '../ui/pages/FileManagerPage'
import FileDetailPage from '../ui/pages/FileDetailPage'
import ContainerPage from '../ui/pages/ContainerPage'
import NetworkPage from '../ui/pages/NetworkPage'
import PlaceholderPage from '../ui/pages/PlaceholderPage'
import UserManagementPage from '../ui/pages/UserManagementPage'
import DappStorePage from '../ui/pages/DappStorePage'
import SettingsPage from '../ui/pages/SettingsPage'
import RecentEventsPage from '../ui/pages/RecentEventsPage'
import SystemLogsPage from '../ui/pages/SystemLogsPage'
import StoragePage from '../ui/pages/StoragePage'
import MessageHubPage from '../ui/pages/MessageHubPage'
import WorkspaceLayout from '../ui/workspace/WorkspaceLayout'

const router = createBrowserRouter([
  {
    path: '/login',
    element: <LoginPage />,
  },
  {
    path: '/sso/login',
    element: <SsoLoginPage />,
  },
  {
    path: '/workspace',
    element: (
      <RequireAuth>
        <WorkspaceLayout />
      </RequireAuth>
    ),
  },
  {
    path: '/',
    children: [
      { path: 'share/:shareId', element: <FileManagerPage /> },
      {
        element: <RequireAuth />,
        children: [
          { index: true, element: <DesktopHomePage /> },
          { path: 'message-hub', element: <Navigate to="/message-hub/today" replace /> },
          { path: 'message-hub/today', element: <MessageHubPage /> },
          { path: 'message-hub/chat', element: <MessageHubPage /> },
          { path: 'message-hub/people', element: <MessageHubPage /> },
          { path: 'message-hub/tasks', element: <MessageHubPage /> },
          { path: 'message-hub/agents', element: <MessageHubPage /> },
          { path: 'files/detail', element: <FileDetailPage /> },
          { path: 'index', element: <Navigate to="/monitor" replace /> },
          { path: 'index.html', element: <Navigate to="/monitor" replace /> },
          {
            element: (
              <RootLayout />
            ),
            children: [
              {
                path: 'monitor',
                element: <DashboardPage />,
              },
              {
                path: 'network',
                element: <NetworkPage />,
              },
              {
                path: '0monitor',
                element: <Navigate to="/monitor" replace />,
              },
              {
                path: 'users',
                element: <UserManagementPage />,
              },
              {
                path: 'storage',
                element: <StoragePage />,
              },
              {
                path: 'containers',
                element: <ContainerPage />,
              },
              {
                path: 'dapps',
                element: <DappStorePage />,
              },
              {
                path: 'settings',
                element: <SettingsPage />,
              },
              {
                path: 'notifications',
                element: <RecentEventsPage />,
              },
              {
                path: 'system-logs',
                element: <SystemLogsPage />,
              },
              {
                path: 'sign-out',
                element: (
                  <PlaceholderPage
                    title="Sign Out"
                    description="To keep your environment secure, make sure you close any open terminals or browser tabs."
                    ctaLabel="Return to Login"
                  />
                ),
              },
            ],
          },
        ],
      },
    ],
  },
  {
    path: '*',
    element: <NotFoundPage />,
  },
])

export default router
