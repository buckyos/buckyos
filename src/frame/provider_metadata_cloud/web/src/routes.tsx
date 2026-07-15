import { createBrowserRouter } from 'react-router-dom'
import { CloudConsoleShell } from './layout/CloudConsoleShell'
import { DashboardPage } from './pages/dashboard'
import { ProvidersPage } from './pages/providers'
import { ModelsPage } from './pages/models'
import { NickRulesPage } from './pages/nick-rules'
import { ResolverRulesPage } from './pages/resolver-rules'
import { LogicalDirectoryPage } from './pages/logical-directory'
import { DictionariesPage } from './pages/dictionaries'
import { ImportPlanPage } from './pages/import-plan'
import { PublishPage } from './pages/publish'
import { ChangeLogsPage } from './pages/change-logs'
import { TechSourcePage } from './pages/tech-source'
import { BulkOperationsPage } from './pages/bulk-operations'
import { WarningsPage } from './pages/warnings'
import { ProviderWizardPage } from './workflows/provider-wizard/ProviderWizard'

export const router = createBrowserRouter([
  {
    path: '/',
    element: <CloudConsoleShell />,
    children: [
      { index: true, element: <DashboardPage /> },
      { path: 'providers/wizard', element: <ProviderWizardPage /> },
      { path: 'providers', element: <ProvidersPage /> },
      { path: 'models', element: <ModelsPage /> },
      { path: 'nick-rules', element: <NickRulesPage /> },
      { path: 'resolver-rules', element: <ResolverRulesPage /> },
      { path: 'logical-directory', element: <LogicalDirectoryPage /> },
      { path: 'dictionaries', element: <DictionariesPage /> },
      { path: 'import-plan', element: <ImportPlanPage /> },
      { path: 'publish', element: <PublishPage /> },
      { path: 'change-logs', element: <ChangeLogsPage /> },
      { path: 'tech-source', element: <TechSourcePage /> },
      { path: 'bulk-operations', element: <BulkOperationsPage /> },
      { path: 'warnings', element: <WarningsPage /> },
    ],
  },
])
