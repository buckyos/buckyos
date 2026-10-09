import type {
  ActiveSession,
  AgentProfile,
  ArtifactHead,
  ArtifactVersion,
  BacklogItem,
  BehaviorMeta,
  Decision,
  Hint,
  IdentityText,
  LoaderStatus,
  PerceptionCursor,
  PerceptionRecord,
  ProfilePatch,
  RegistryEntry,
  SessionDetail,
  UiBinding,
  UsageByModel,
} from './types'

export interface PerceptionView {
  cursor: PerceptionCursor
  backlog: BacklogItem[]
}

export interface BehaviorCatalog {
  revision: string
  behaviors: BehaviorMeta[]
}

export interface OpenDanDataModel {
  readonly source: 'mock' | 'krpc'

  loaderStatus(): Promise<LoaderStatus>

  profile(): Promise<AgentProfile>
  setProfile(patch: ProfilePatch): Promise<AgentProfile>
  /** `null`: the service does not record usage by model. */
  usageModels(): Promise<UsageByModel | null>
  uiBindings(): Promise<UiBinding[]>

  sessions(): Promise<RegistryEntry[]>
  activeSessions(): Promise<ActiveSession[]>
  session(sid: string, worklog?: number): Promise<SessionDetail>

  stopSession(sid: string, reason?: string): Promise<{ index: number }>
  decideSession(sid: string, decision: Decision, note?: string): Promise<{ index: number }>
  postMessage(sid: string, text: string): Promise<{ index: number; key: string }>

  perception(): Promise<PerceptionView>
  perceptionRecords(item: BacklogItem): Promise<PerceptionRecord[]>
  artifacts(): Promise<ArtifactHead[]>
  artifactVersions(aid: string): Promise<ArtifactVersion[]>
  behaviors(): Promise<BehaviorCatalog>
  behavior(name: string): Promise<unknown>
  identity(): Promise<IdentityText>
  recallHints(tags: string[], maxHints?: number): Promise<Hint[]>
}

export class OpenDanError extends Error {
  readonly kind: string

  constructor(kind: string, message: string) {
    super(message)
    this.kind = kind
  }
}
