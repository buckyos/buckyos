import { createBrowserRouter } from 'react-router-dom'
import RootLayout from '../ui/RootLayout'
import DashboardPage from '../ui/pages/DashboardPage'
import PlaceholderPage from '../ui/pages/PlaceholderPage'

const router = createBrowserRouter([
  {
    path: '/',
    element: <RootLayout />,
    children: [
      { index: true, element: <DashboardPage /> },
      {
        path: 'users',
        element: (
          <PlaceholderPage
            title="User Management"
            description="Manage user roles, invitations, and access policies for your BuckyOS zone."
            ctaLabel="Invite User"
          />
        ),
      },
      {
        path: 'storage',
        element: (
          <PlaceholderPage
            title="Storage"
            description="Inspect capacity usage, configure replication policies, and manage snapshots."
            ctaLabel="Open Storage Console"
          />
        ),
      },
      {
        path: 'dapps',
        element: (
          <PlaceholderPage
            title="dApp Store"
            description="Discover, install, and keep your decentralized applications up to date."
            ctaLabel="Browse dApps"
          />
        ),
      },
      {
        path: 'settings',
        element: (
          <PlaceholderPage
            title="Settings"
            description="Adjust system preferences, security hardening, and advanced node configuration."
            ctaLabel="Update Settings"
          />
        ),
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
