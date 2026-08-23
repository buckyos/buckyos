import type { AppSummary } from '../api/app_mgr'
import type { AppDefinition } from '../models/ui'

export const DESKTOP_BUILTIN_APP_IDS = new Set([
  'ai-center',
  'files',
  'task-center',
  'workflow',
  'settings',
  'diagnostics',
  'users-agents',
  'my-network',
  'app-service',
])

const logicalAppAliases: Readonly<Record<string, string>> = {
  'content-store': 'market',
  'buckyos-systest.buckyos.bns.did': 'systest',
}

export function desktopCatalogIdForLogicalApp(appId: string): string {
  return logicalAppAliases[appId] ?? appId
}

function normalizedWebHosts(summary: AppSummary): string[] {
  return [
    ...new Set(summary.web_hosts.map((host) => host.trim()).filter(Boolean)),
  ]
}

export function createBackendAppDefinitionMapper(catalog: AppDefinition[]) {
  const catalogById = new Map(catalog.map((app) => [app.id, app]))

  return (summary: AppSummary): AppDefinition => {
    const webHosts = normalizedWebHosts(summary)
    const catalogEntry = catalogById.get(
      desktopCatalogIdForLogicalApp(summary.app_id),
    )
    const showName = summary.show_name?.trim()
    const displayName = showName && showName !== summary.app_instance_id
      ? showName
      : webHosts[0] ?? summary.app_id
    const fallback: AppDefinition = {
      id: summary.app_instance_id,
      iconKey: summary.app_id,
      labelKey: displayName,
      summaryKey: displayName,
      accent: 'var(--cp-accent)',
      tier: webHosts.length > 0 ? 'sdk' : 'external',
      manifest: {
        defaultMode: 'windowed',
        allowMinimize: true,
        allowMaximize: true,
        allowClose: true,
        allowFullscreen: true,
        mobileFullscreenBehavior: 'cover_dead_zone',
        mobileStatusBarMode: 'standard',
        titleBarMode: 'system',
        placement: webHosts.length > 0 ? 'inplace' : 'new-container',
        ...(webHosts.length > 0 ? { contentPadding: 'none' as const } : {}),
      },
    }
    const definition = catalogEntry ?? fallback
    return {
      ...definition,
      id: summary.app_instance_id,
      logicalAppId: summary.app_id,
      appInstanceId: summary.app_instance_id,
      ownerUserId: summary.owner_user_id,
      webHosts,
      ...(webHosts.length > 0
        ? {
          tier: 'sdk' as const,
          manifest: {
            ...definition.manifest,
            placement: 'inplace' as const,
            contentPadding: 'none' as const,
          },
        }
        : {}),
    }
  }
}

export function buildAuthorizedAppDefinitions(
  catalog: AppDefinition[],
  authorizedApps: AppSummary[],
): AppDefinition[] {
  const toDefinition = createBackendAppDefinitionMapper(catalog)
  const desktopBuiltInApps = catalog.filter((app) => DESKTOP_BUILTIN_APP_IDS.has(app.id))
  return [...desktopBuiltInApps, ...authorizedApps.map(toDefinition)]
}
