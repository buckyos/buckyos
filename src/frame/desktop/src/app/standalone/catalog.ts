/* ── The system apps that have a page of their own (system-apps.json lists the same ones) ── */

import { buildDesktopPayload } from '../../mock/data'
import type { AppDefinition } from '../../models/ui'
import { AGENT_GUIDE_APP_ID, AGENT_SETUP_APP_ID } from '../agent-setup/model'
import { DESKTOP_BUILTIN_APP_IDS } from '../backend-apps'

/** Diagnostics shows the desktop's own layout and activity: without a desktop there is nothing to show. */
const DESKTOP_ONLY = new Set(['diagnostics'])

/** Who the desktop offers the app to: Agents are created by some kinds of users only. */
export type SystemAppAudience = 'all' | 'agent_user'
const AGENT_USER_APPS = new Set([AGENT_GUIDE_APP_ID, AGENT_SETUP_APP_ID])

export function hasStandalonePage(appId: string): boolean {
  return DESKTOP_BUILTIN_APP_IDS.has(appId) && !DESKTOP_ONLY.has(appId)
}

/** The built-in catalog's apps with a page, as the desktop defines them. */
export function systemApps(): AppDefinition[] {
  return buildDesktopPayload('desktop').apps.filter((app) => hasStandalonePage(app.id))
}

export function systemAppAudience(appId: string): SystemAppAudience {
  return AGENT_USER_APPS.has(appId) ? 'agent_user' : 'all'
}
