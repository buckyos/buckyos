import type {
  ActiveSession,
  AgentProfile,
  ArtifactHead,
  ArtifactVersion,
  BehaviorMeta,
  Decision,
  GoalActivation,
  GoalChange,
  GoalDefinition,
  GoalSchedule,
  GoalState,
  GoalStatus,
  MemoryQueryResult,
  IdentityText,
  InstallPlan,
  KnownRuntime,
  RuntimeExtTools,
  KnownWorkspace,
  LoaderStatus,
  ProfilePatch,
  RegistryEntry,
  RuntimeSettingPatch,
  RuntimeSettings,
  SessionDetail,
  SkillDraft,
  SkillImportRequest,
  SkillInstallState,
  SkillKind,
  SkillMeta,
  SkillSource,
  SkillsState,
  SkillsStatus,
  UiBinding,
  UiSkills,
  UsageByModel,
  WorkspaceCreateRequest,
  WorkspaceImportRequest,
  WorkspaceLocation,
} from './types'

export interface BehaviorCatalog {
  revision: string
  behaviors: BehaviorMeta[]
}

/** Whether the page still has the zone login its calls need. */
export interface LoginState {
  /** `true` once the zone turns this page away: only a new sign-in helps. */
  lost(): boolean
  subscribe(notify: () => void): () => void
}

export interface OpenDanDataModel {
  readonly source: 'mock' | 'krpc'
  readonly login: LoginState

  loaderStatus(): Promise<LoaderStatus>

  profile(): Promise<AgentProfile>
  setProfile(patch: ProfilePatch): Promise<AgentProfile>
  /** `null`: the service does not record usage by model. */
  usageModels(): Promise<UsageByModel | null>
  uiBindings(): Promise<UiBinding[]>
  workspaces(): Promise<KnownWorkspace[]>
  runtimes(): Promise<KnownRuntime[]>
  checkWorkspace(workspaceId: string): Promise<KnownWorkspace>
  relocateWorkspace(workspace: KnownWorkspace, location: WorkspaceLocation): Promise<KnownWorkspace>
  createWorkspace(request: WorkspaceCreateRequest): Promise<KnownWorkspace>
  importWorkspace(request: WorkspaceImportRequest): Promise<KnownWorkspace>
  /** `''` clears the project URL. */
  setWorkspaceUrl(workspace: KnownWorkspace, url: string): Promise<KnownWorkspace>
  checkRuntime(runtimeId: string): Promise<KnownRuntime>
  runtimeExtTools(runtimeId: string): Promise<RuntimeExtTools>
  runtimeSettings(): Promise<RuntimeSettings>
  setRuntimeSetting(patch: RuntimeSettingPatch): Promise<RuntimeSettings>

  sessions(): Promise<RegistryEntry[]>
  activeSessions(): Promise<ActiveSession[]>
  session(sid: string, worklog?: number): Promise<SessionDetail>

  stopSession(sid: string, reason?: string): Promise<{ index: number }>
  decideSession(sid: string, decision: Decision, note?: string): Promise<{ index: number }>
  postMessage(sid: string, text: string): Promise<{ index: number; key: string }>

  artifacts(): Promise<ArtifactHead[]>
  artifactVersions(aid: string): Promise<ArtifactVersion[]>
  behaviors(): Promise<BehaviorCatalog>
  behavior(name: string): Promise<unknown>
  identity(): Promise<IdentityText>
  /** What session `session` finds in Memory for `words` (its grants decide what is visible). */
  queryMemory(session: string, words: string[]): Promise<MemoryQueryResult>

  goals(): Promise<GoalDefinition[]>
  /** `null`: no such Goal (or revision). */
  goal(goalId: string): Promise<GoalDefinition | null>
  goalRevision(goalId: string, revision: number): Promise<GoalDefinition | null>
  /** The newest `limit` changes, oldest first (0: all). */
  goalChanges(goalId: string, limit?: number): Promise<GoalChange[]>
  goalState(goalId: string): Promise<GoalState | null>
  goalSchedule(goalId: string): Promise<GoalSchedule | null>
  /** The newest `limit` Activations (of one epoch), oldest first. */
  goalActivations(goalId: string, epoch?: number, limit?: number): Promise<GoalActivation[]>
  activateGoal(goalId: string, reason?: string): Promise<{ index: number; key: string }>
  setGoalStatus(goalId: string, status: GoalStatus, reason?: string): Promise<GoalDefinition>
  reopenGoal(goalId: string, reason: string, freshState?: boolean): Promise<GoalDefinition>
  restartGoal(goalId: string, reason: string, carryState?: boolean): Promise<GoalDefinition>

  skills(): Promise<SkillMeta[]>
  skillsState(): Promise<SkillsState>
  uiSkills(): Promise<UiSkills>
  /** Probes the runtime (default: the private one): it can take seconds. */
  skillsStatus(runtimeId?: string): Promise<SkillsStatus>
  /** What installing would do; nothing is written. */
  skillInstallPlan(source: SkillSource, runtimeId?: string): Promise<InstallPlan>
  skillDrafts(): Promise<SkillDraft[]>
  skillDraftFile(slug: string, file: string): Promise<{ text: string }>
  // Owner only.
  installSkills(source: SkillSource, planDigest: string, runtimeId?: string, replace?: boolean): Promise<unknown>
  removeSkill(kind: SkillKind, name: string): Promise<unknown>
  setSkillEnabled(kind: SkillKind, name: string, enabled: boolean, reason?: string): Promise<SkillMeta>
  /** `null`: back to the agent package's set. */
  setUiSkills(names: string[] | null): Promise<UiSkills>
  installSkillCommands(name: string, runtimeId: string): Promise<SkillInstallState>
  checkSkillCommands(name: string, runtimeId?: string): Promise<SkillInstallState>
  /** The agent writes a draft from the text; it appears in `skillDrafts` when the session is done. */
  importSkill(request: SkillImportRequest): Promise<{ session_id: string; inbox?: unknown }>
  discardSkillDraft(slug: string): Promise<unknown>
}

export class OpenDanError extends Error {
  readonly kind: string

  constructor(kind: string, message: string) {
    super(message)
    this.kind = kind
  }
}
