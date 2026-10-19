import type { AppSummary } from '../api/app_mgr'
import type { AppDefinition, AppDisplayText } from '../models/ui'
import { isLauncherApp } from '../models/ui'
import { DEFAULT_PRESENTATION_LOCALE, didDisplayName, firstText, pickLocalized, resolveImageReference } from '../presentation/model'

export const DESKTOP_BUILTIN_APP_IDS = new Set([
  'ai-center',
  'files',
  'messagehub',
  'task-center',
  'workflow',
  'settings',
  'diagnostics',
  'users-agents',
  'my-network',
  'my-content',
  'app-service',
  'preview',
  'aiworkspace',
  'homestation',
  'agent-setup',
  'agent-guide',
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

function localizedOrUndefined(value: Record<string, string> | undefined) {
  return value && pickLocalized(value, DEFAULT_PRESENTATION_LOCALE) ? value : undefined
}

/**
 * The app's own document text: `presentation.title` (per language) → `show_name` → package name →
 * the DID's readable part → AppId. A catalog app keeps its built-in i18n text when the document has none.
 */
export function backendAppText(summary: AppSummary, inCatalog: boolean): AppDisplayText {
  const presentation = summary.presentation ?? undefined
  const did = summary.app_did?.trim()
  const title = localizedOrUndefined(presentation?.title)
    ?? firstText(summary.show_name, summary.name)
    ?? (inCatalog ? undefined : firstText(did && didDisplayName(did), summary.app_id))
  const description = localizedOrUndefined(presentation?.summary) ?? localizedOrUndefined(presentation?.description)
  return {
    ...(title ? { title } : {}),
    ...(description ? { summary: description } : {}),
  }
}

export interface BackendAppMapperOptions {
  /** The Zone host; with an app's first web host it is the base of a relative icon reference. */
  zoneHost?: string
  protocol?: string
}

export function createBackendAppDefinitionMapper(catalog: AppDefinition[], options: BackendAppMapperOptions = {}) {
  const catalogById = new Map(catalog.map((app) => [app.id, app]))

  return (summary: AppSummary): AppDefinition => {
    const webHosts = normalizedWebHosts(summary)
    const catalogEntry = catalogById.get(
      desktopCatalogIdForLogicalApp(summary.app_id),
    )
    const fallback: AppDefinition = {
      id: summary.app_instance_id,
      iconKey: summary.app_id,
      labelKey: summary.app_id,
      summaryKey: '',
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
    const text = backendAppText(summary, Boolean(catalogEntry))
    const iconBase = webHosts[0] && options.zoneHost
      ? `${options.protocol ?? 'https:'}//${webHosts[0]}.${options.zoneHost}/`
      : undefined
    const iconUrl = resolveImageReference(summary.app_icon_url, iconBase) ?? definition.iconUrl
    // An Agent's constructed App opens from that Agent's entries, never as a launcher icon.
    const hiddenFromLauncher = summary.runtime_type === 'agent'
    return {
      ...definition,
      ...(hiddenFromLauncher ? { manifest: { ...definition.manifest, showInLauncher: false } } : {}),
      id: summary.app_instance_id,
      logicalAppId: summary.app_id,
      appInstanceId: summary.app_instance_id,
      ownerUserId: summary.owner_user_id,
      ...(summary.app_did?.trim() ? { did: summary.app_did.trim() } : {}),
      ...(Object.keys(text).length > 0 ? { text } : {}),
      ...(iconUrl ? { iconUrl } : {}),
      webHosts,
      ...(webHosts.length > 0
        ? {
          tier: 'sdk' as const,
          manifest: {
            ...definition.manifest,
            ...(hiddenFromLauncher ? { showInLauncher: false } : {}),
            placement: 'inplace' as const,
            contentPadding: 'none' as const,
          },
        }
        : {}),
    }
  }
}

/** Built-in desktop items that depend on the account: the Jarvis guide is only for users who can create Agents. */
export function withAccountBuiltins(apps: AppDefinition[], options: { agentGuide: boolean }): AppDefinition[] {
  return options.agentGuide ? apps : apps.filter((app) => app.id !== 'agent-guide')
}

export function buildAuthorizedAppDefinitions(
  catalog: AppDefinition[],
  authorizedApps: AppSummary[],
  options: BackendAppMapperOptions = {},
): AppDefinition[] {
  const toDefinition = createBackendAppDefinitionMapper(catalog, options)
  const desktopBuiltInApps = catalog.filter((app) => DESKTOP_BUILTIN_APP_IDS.has(app.id))
  return [...desktopBuiltInApps, ...authorizedApps.map(toDefinition)]
}

/** Launcher items only: Apps hidden from the launcher keep their definition but get no desktop icon. */
export function launcherDefinitions(definitions: AppDefinition[]): AppDefinition[] {
  return definitions.filter(isLauncherApp)
}
