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
  Hint,
  IdentityText,
  KnownRuntime,
  KnownWorkspace,
  LoaderStatus,
  ProfilePatch,
  RegistryEntry,
  SessionDetail,
  UiBinding,
  UsageByModel,
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
  recallHints(tags: string[], maxHints?: number): Promise<Hint[]>

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
}

export class OpenDanError extends Error {
  readonly kind: string

  constructor(kind: string, message: string) {
    super(message)
    this.kind = kind
  }
}
