import { createBrowserRouter } from 'react-router-dom'
import RootLayout from '../ui/RootLayout'
import DashboardPage from '../ui/pages/DashboardPage'
import StoragePage from '../ui/pages/StoragePage'
import PlaceholderPage from '../ui/pages/PlaceholderPage'
import UserManagementPage from '../ui/pages/UserManagementPage'
import DappStorePage from '../ui/pages/DappStorePage'
import SettingsPage from '../ui/pages/SettingsPage'

const router = createBrowserRouter([
  {
    path: '/',
    element: <RootLayout />,
    children: [
      { index: true, element: <DashboardPage /> },
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
        element: (
          <PlaceholderPage
            title="Notifications"
            description="Review alerts, scheduled tasks, and automation rules triggered by your infrastructure."
            ctaLabel="Review Alerts"
          />
        ),
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
])

export default router
