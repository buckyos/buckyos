import { Navigate, createBrowserRouter } from 'react-router-dom'
import RootLayout from '../ui/RootLayout'
import DashboardPage from '../ui/pages/DashboardPage'
import DesktopHomePage from '../ui/pages/DesktopHomePage'
import LoginPage from '../ui/pages/LoginPage'
import StoragePage from '../ui/pages/StoragePage'
import PlaceholderPage from '../ui/pages/PlaceholderPage'
import UserManagementPage from '../ui/pages/UserManagementPage'
import DappStorePage from '../ui/pages/DappStorePage'
import SettingsPage from '../ui/pages/SettingsPage'
import RecentEventsPage from '../ui/pages/RecentEventsPage'
import SystemLogsPage from '../ui/pages/SystemLogsPage'

const router = createBrowserRouter([
  {
    path: '/login',
    element: <LoginPage />,
  },
  {
    path: '/login.html',
    element: <LoginPage />,
  },
  {
    path: '/',
    children: [
      { index: true, element: <DesktopHomePage /> },
      { path: 'index', element: <Navigate to="/monitor" replace /> },
      { path: 'index.html', element: <Navigate to="/monitor" replace /> },
      {
        element: <RootLayout />,
        children: [
          {
            path: 'monitor',
            element: <DashboardPage />,
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
])

export default router
