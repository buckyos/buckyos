/* ── Agent setup – pure datamodel (no runtime / SDK imports, Deno-testable) ── */

import type {
  AgentCreateRequest,
  AgentCreateStep,
  AgentEntry,
  AgentStatus,
  AgentTemplate,
  UserDetail,
  UserTunnelBinding,
} from '../../api/user_mgr.ts'

export const AGENT_SETUP_APP_ID = 'agent-setup'
export const AGENT_GUIDE_APP_ID = 'agent-guide'
export const JARVIS_GUIDE_ENTRY = 'jarvis_guide'
export const AGENT_LOADER_OPENDAN = 'opendan'

export type AgentSetupSource = 'jarvis_guide' | 'users_agents'
export type AgentSetupPage = 'account' | 'permissions' | 'runtime' | 'channel' | 'confirm'
export type AgentSetupChannel = 'none' | 'telegram'

export const AGENT_SETUP_PAGES: readonly AgentSetupPage[] = ['account', 'permissions', 'runtime', 'channel', 'confirm']

/** Display name rule shared by every surface: a non-empty nickname, otherwise the user name. */
export function agentDisplayName(entry: Pick<AgentEntry, 'name'> & Partial<Pick<AgentEntry, 'display_name' | 'profile'>>): string {
  return entry.profile?.display_name?.trim() || entry.display_name?.trim() || entry.name
}

// ── Desktop guide entry (PRD §9.4) ──

export type AgentGuideState =
  | { kind: 'none' }
  | { kind: 'creating'; agent: AgentEntry }
  | { kind: 'failed'; agent: AgentEntry }
  | { kind: 'linked'; agent: AgentEntry }

const guidePriority: Record<string, number> = { ready: 0, bound: 1, provisioning: 2, failed: 3 }

/** The caller's own Agent created from the desktop guide decides what the guide shows and opens. */
export function computeAgentGuideState(agents: readonly AgentEntry[], userId: string): AgentGuideState {
  const candidates = agents
    .filter((agent) =>
      agent.owner_user_id === userId &&
      agent.settings?.desktop_entry === JARVIS_GUIDE_ENTRY &&
      agent.install?.state in guidePriority,
    )
    .sort((a, b) =>
      guidePriority[a.install.state] - guidePriority[b.install.state] ||
      (b.install.created_at ?? 0) - (a.install.created_at ?? 0),
    )
  const agent = candidates[0]
  if (!agent) return { kind: 'none' }
  if (agent.install.state === 'ready') return { kind: 'linked', agent }
  if (agent.install.state === 'failed') return { kind: 'failed', agent }
  return { kind: 'creating', agent }
}

/** Where an Agent's home is: the constructed App's Web entry, else its Users and Agents details. */
export function agentHomeTarget(entry: AgentEntry): { kind: 'app'; appId: string } | { kind: 'details' } {
  if (!entry.runtime?.has_web) return { kind: 'details' }
  return { kind: 'app', appId: entry.runtime.app_instance_id || `${entry.agent_id}@${entry.owner_user_id}` }
}

/** Avatars are data URLs (or http(s) for template icons); anything else is not shown. */
export function usableImageUrl(value: string | null | undefined): string | undefined {
  const url = value?.trim()
  if (!url) return undefined
  return /^(data:image\/|https?:\/\/|\/)/i.test(url) ? url : undefined
}

// ── Errors ──

export type AgentErrorKind =
  | 'limited_user'
  | 'name_conflict'
  | 'invalid_name'
  | 'sharing_unsupported'
  | 'owner_identity_missing'
  | 'template_unavailable'
  | 'invalid_bot_token'
  | 'idempotency_conflict'
  | 'agent_busy'
  | 'cancel_unavailable'
  | 'not_found'
  | 'session_expired'
  | 'network'
  | 'other'

export interface ClassifiedAgentError {
  kind: AgentErrorKind
  detail: string
}

const errorPrefixes: Array<[AgentErrorKind, RegExp]> = [
  ['limited_user', /limited_user:\s*(.*)$/is],
  ['name_conflict', /name_conflict:\s*(.*)$/is],
  ['invalid_name', /invalid_name:\s*(.*)$/is],
  ['sharing_unsupported', /sharing_unsupported:\s*(.*)$/is],
  ['owner_identity_missing', /owner_identity_missing:\s*(.*)$/is],
  ['template_unavailable', /template_(?:not_found|invalid|unavailable):\s*(.*)$/is],
  ['invalid_bot_token', /invalid_bot_token:\s*(.*)$/is],
  ['idempotency_conflict', /idempotency_conflict:\s*(.*)$/is],
  ['agent_busy', /agent_busy:\s*(.*)$/is],
  ['cancel_unavailable', /cancel_unavailable:\s*(.*)$/is],
  ['not_found', /agent_not_found:\s*(.*)$/is],
]

export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  if (error && typeof error === 'object' && 'message' in error) return String((error as { message: unknown }).message)
  return String(error ?? '')
}

export function classifyAgentError(error: unknown): ClassifiedAgentError {
  const message = errorMessage(error)
  for (const [kind, pattern] of errorPrefixes) {
    const match = pattern.exec(message)
    if (match) return { kind, detail: match[1].trim() }
  }
  if (/token expired|invalid token|session (?:has )?expired|not logged in|RPC call error: 401\b/i.test(message)) {
    return { kind: 'session_expired', detail: message }
  }
  if (/RPC call failed|failed to fetch|networkerror|network error|load failed|timed? ?out|RPC call error: 50[234]\b/i.test(message)) {
    return { kind: 'network', detail: message }
  }
  if (/\bnot found\b/i.test(message) && /agent/i.test(message)) return { kind: 'not_found', detail: message }
  return { kind: 'other', detail: message.replace(/^RPC call error:\s*(?:Failed due to reason:\s*)?/i, '') }
}

// ── Identity ──

const DNS_LABEL = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/

export function normalizeAgentNameInput(value: string): string {
  return value.toLowerCase().replace(/\s+/g, '')
}

export function isValidAgentName(name: string): boolean {
  return DNS_LABEL.test(name)
}

export const TELEGRAM_BOT_TOKEN_PATTERN = /^\d+:[A-Za-z0-9_-]{30,}$/

export function isTelegramBotToken(value: string): boolean {
  return TELEGRAM_BOT_TOKEN_PATTERN.test(value.trim())
}

/** A bare Telegram user id (digits), as stored in the Owner's profile. */
export function isTelegramAccountId(value: string): boolean {
  return /^\d{3,20}$/.test(value.trim())
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
}

/** The Owner's own Telegram identity in `local_profile.private_extra.system_contact.bindings`. */
export function ownerChannelBinding(detail: UserDetail | null | undefined, platform: string): UserTunnelBinding | null {
  const contact = asRecord(asRecord(asRecord(detail?.local_profile).private_extra).system_contact)
  const bindings = Array.isArray(contact.bindings) ? contact.bindings : []
  for (const item of bindings) {
    const binding = asRecord(item)
    if (binding.platform === platform && typeof binding.account_id === 'string' && binding.account_id.trim()) {
      return binding as unknown as UserTunnelBinding
    }
  }
  return null
}

// ── Creation progress ──

export type CreationStepKey = Exclude<AgentCreateStep, 'done'>
export type CreationStepPhase = 'done' | 'active' | 'failed' | 'pending' | 'skipped'

export function creationStepPhases(status: AgentStatus): Array<{ step: CreationStepKey; phase: CreationStepPhase }> {
  const steps: CreationStepKey[] = status.tunnel_state === 'none'
    ? ['runtime', 'bind', 'start']
    : ['runtime', 'bind', 'tunnel', 'start']
  const phaseOf = (step: CreationStepKey, index: number): CreationStepPhase => {
    if (step === 'tunnel' && status.tunnel_state === 'skipped') return 'skipped'
    if (status.state === 'ready') return 'done'
    const current = status.state === 'failed' ? status.last_error?.step ?? status.step : status.step
    const currentIndex = current === 'done' ? steps.length : steps.indexOf(current as CreationStepKey)
    if (index < currentIndex) return 'done'
    if (index > currentIndex) return 'pending'
    return status.state === 'failed' ? 'failed' : 'active'
  }
  return steps.map((step, index) => ({ step, phase: phaseOf(step, index) }))
}

export function isCreationInProgress(status: AgentStatus): boolean {
  return status.state === 'provisioning' || status.state === 'bound'
}

/** `agent.create.cancel` only works before the spec is written (runtime / bind). */
export function canCancelCreation(status: AgentStatus): boolean {
  if (status.state === 'provisioning') return true
  if (status.state !== 'failed') return false
  const step = status.last_error?.step ?? status.step
  return step === 'runtime' || step === 'bind'
}

/** Telegram failures the channel step reports in `last_error.code`. */
export function tunnelFailureKey(code: string | null | undefined): string | null {
  if (code === 'telegram_bot_invalid') return 'agentSetup.status.telegramBotInvalid'
  if (code === 'telegram_unreachable') return 'agentSetup.status.telegramUnreachable'
  return null
}

/** Seconds or milliseconds since the epoch, as a Date. */
export function statusTime(value: number | null | undefined): Date | null {
  if (!value || value <= 0) return null
  return new Date(value > 10_000_000_000 ? value : value * 1000)
}

// ── Draft ──

export interface AgentSetupDraft {
  version: 1
  source: AgentSetupSource
  idempotencyKey: string
  page: AgentSetupPage
  name: string
  nameEdited: boolean
  displayName: string
  avatar: string | null
  bio: string
  bioEdited: boolean
  roleSupplement: string
  allowGroup: boolean
  templateId: string | null
  templateAutoUpdate: boolean
  channel: AgentSetupChannel
  submittedAgentId: string | null
}

export function agentSetupDraftKey(userId: string, source: AgentSetupSource): string {
  return `buckyos.agent-setup.draft.v1.${encodeURIComponent(userId)}.${source}`
}

export function newAgentSetupDraft(source: AgentSetupSource, idempotencyKey: string): AgentSetupDraft {
  const fromGuide = source === 'jarvis_guide'
  return {
    version: 1,
    source,
    idempotencyKey,
    page: 'account',
    name: fromGuide ? 'jarvis' : '',
    nameEdited: false,
    displayName: fromGuide ? 'Jarvis' : '',
    avatar: null,
    bio: '',
    bioEdited: false,
    roleSupplement: '',
    allowGroup: false,
    templateId: null,
    templateAutoUpdate: true,
    channel: 'none',
    submittedAgentId: null,
  }
}

function stringField(value: unknown, fallback: string): string {
  return typeof value === 'string' ? value : fallback
}

function boolField(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback
}

/** A stored draft, or null when it is missing, foreign or malformed. */
export function parseAgentSetupDraft(raw: unknown, source: AgentSetupSource): AgentSetupDraft | null {
  const value = asRecord(raw)
  if (value.version !== 1 || value.source !== source || typeof value.idempotencyKey !== 'string' || !value.idempotencyKey) return null
  const base = newAgentSetupDraft(source, value.idempotencyKey)
  const page = AGENT_SETUP_PAGES.includes(value.page as AgentSetupPage) ? value.page as AgentSetupPage : base.page
  return {
    ...base,
    page,
    name: stringField(value.name, base.name),
    nameEdited: boolField(value.nameEdited, base.nameEdited),
    displayName: stringField(value.displayName, base.displayName),
    avatar: typeof value.avatar === 'string' && value.avatar.startsWith('data:image/') ? value.avatar : null,
    bio: stringField(value.bio, base.bio),
    bioEdited: boolField(value.bioEdited, base.bioEdited),
    roleSupplement: stringField(value.roleSupplement, base.roleSupplement),
    allowGroup: boolField(value.allowGroup, base.allowGroup),
    templateId: typeof value.templateId === 'string' && value.templateId ? value.templateId : null,
    templateAutoUpdate: boolField(value.templateAutoUpdate, base.templateAutoUpdate),
    channel: value.channel === 'telegram' ? 'telegram' : 'none',
    submittedAgentId: typeof value.submittedAgentId === 'string' && value.submittedAgentId ? value.submittedAgentId : null,
  }
}

export function draftDisplayName(draft: Pick<AgentSetupDraft, 'displayName' | 'name'>): string {
  return draft.displayName.trim() || draft.name.trim()
}

/** Compatible templates of a Loader; the default one is preferred. */
export function templatesForLoader(templates: readonly AgentTemplate[], loader: string): AgentTemplate[] {
  return templates.filter((template) => template.loader === loader)
}

export function pickTemplate(templates: readonly AgentTemplate[], templateId: string | null): AgentTemplate | null {
  return templates.find((template) => template.template_id === templateId)
    ?? templates.find((template) => template.is_default)
    ?? templates[0]
    ?? null
}

/**
 * Applies a template's defaults without touching what the user edited: only an
 * untouched public bio follows the template's description.
 */
export function applyTemplateToDraft(draft: AgentSetupDraft, template: AgentTemplate): AgentSetupDraft {
  return {
    ...draft,
    templateId: template.template_id,
    bio: draft.bioEdited ? draft.bio : template.description,
  }
}

export function buildAgentCreateRequest(draft: AgentSetupDraft, botToken: string): AgentCreateRequest {
  const displayName = draft.displayName.trim()
  const bio = draft.bio.trim()
  return {
    idempotency_key: draft.idempotencyKey,
    name: draft.name.trim(),
    profile: {
      ...(displayName ? { display_name: displayName } : {}),
      ...(draft.avatar ? { avatar: draft.avatar } : {}),
      ...(bio ? { bio } : {}),
    },
    role_supplement: draft.roleSupplement.trim(),
    allow_group: draft.allowGroup,
    allow_other_users: false,
    template_id: draft.templateId ?? '',
    template_auto_update: draft.templateAutoUpdate,
    ...(draft.source === 'jarvis_guide' ? { desktop_entry: 'jarvis_guide' as const } : {}),
    ...(draft.channel === 'telegram' ? { msg_tunnel: { platform: 'telegram' as const, bot_token: botToken.trim() } } : {}),
  }
}
