export type AppServiceStatus =
  | 'running'
  | 'starting'
  | 'stopped'
  | 'error'
  | 'installing'
  | 'activation_failed'

export type ServiceLayer = 'app' | 'system' | 'kernel'
export type AppServiceViewStatus = 'loading' | 'ready' | 'error'

export type DockerEngineStatus = 'running' | 'not_running'
export type ImageStatus = 'present' | 'missing' | 'pulling'
export type ContainerStatus = 'running' | 'stopped' | 'error' | 'not_created'

export interface DockerDependency {
  engine: DockerEngineStatus
  image: ImageStatus
  imageName: string
  container: ContainerStatus
}

export interface AppServiceItem {
  id: string
  name: string
  description: string
  iconKey: string
  version: string
  layer: ServiceLayer
  status: AppServiceStatus
  docker: DockerDependency | null
  diagnostics: string[]
  spec: Record<string, string>
  settings: Record<string, string>
  serviceInfo: Record<string, string>
  logs: string[]
  installProgress?: number
  installTaskId?: string
}

export type InstallSourceKind =
  | 'url-app-meta'
  | 'url-pikg'
  | 'app-did'
  | 'app-name'
  | 'app-document-object'
  | 'pikg-object'
  | 'share-object'
  | 'signed-jwt'
  | 'unsigned-json'
  | 'local-pikg'
  | 'personal-server-pikg'

export type NormalizedInstallInputType = 'identifier' | 'staging_handle'

export interface PickedPikgFile {
  location: 'device' | 'personal-server'
  name: string
  sizeBytes: number
}

export interface InstallSourceResolution {
  kind: InstallSourceKind
  originalInput: string
  displaySource: string
  normalizedType: NormalizedInstallInputType
  normalizedValue: string
  fileName?: string
  warningCode?: 'UNSIGNED_CANDIDATE'
}

export type SourceParseErrorCode =
  | 'EMPTY_INPUT'
  | 'INVALID_URL'
  | 'UNSUPPORTED_URL_CONTENT'
  | 'INVALID_APP_META'
  | 'INVALID_PIKG'
  | 'UNRECOGNIZED_INPUT'

export type SourceParseResult =
  | { ok: true; source: InstallSourceResolution }
  | { ok: false; code: SourceParseErrorCode }

export type TrustCheckCode = 'document' | 'signature' | 'owner' | 'authority'
export type TrustCheckStatus = 'verified' | 'warning' | 'pending' | 'failed' | 'unknown'

export interface TrustCheck {
  code: TrustCheckCode
  status: TrustCheckStatus
  detail: string
}

export interface ContentReadiness {
  offlineReady: boolean
  missingBytes: number
  availableSource: string
}

export type InstallPermissionKind = 'files' | 'network' | 'database' | 'system'

export interface InstallPermission {
  kind: InstallPermissionKind
  scope: string
}

export interface InstallAppInfo {
  id: string
  name: string
  version: string
  releaseVersion: string
  description: string
  iconKey: string
  appDid: string
  documentObjectId: string
  publisher: string
  referrer: string
  source: InstallSourceResolution
  trustChecks: TrustCheck[]
  platformSupported: boolean
  content: ContentReadiness
  permissions: InstallPermission[]
  availableComponents: string[]
  defaultSettings: Record<string, string>
  installReady: boolean
  blockingReason?:
    | 'TRUST_RESOLUTION_REQUIRED'
    | 'IDENTITY_REVOKED'
    | 'TARGET_UNSUPPORTED'
    | 'OFFLINE_CONTENT_UNAVAILABLE'
}

export type InstallTargetNode = 'ood-primary' | 'ood-backup'
export type InstallNetworkMode = 'private' | 'zone'

export interface InstallOptions {
  targetNode: InstallTargetNode
  components: string[]
  dataDir: string
  networkMode: InstallNetworkMode
  autoStart: boolean
}

export interface InstallPlan {
  options: InstallOptions
  permissions: InstallPermission[]
  impacts: Array<'container' | 'persistent-data' | 'network-route'>
  content: ContentReadiness
  ready: boolean
}

export type InstallTaskStage =
  | 'resolve'
  | 'inspect'
  | 'acquire'
  | 'verify'
  | 'prepare'
  | 'deploy'
  | 'activate'
  | 'completed'

export type InstallTaskStatus = 'waiting_for_approval' | 'running' | 'completed' | 'failed'

export interface InstallTaskHistoryItem {
  stage: InstallTaskStage
  status: 'completed' | 'current' | 'skipped'
}

export interface InstallFailure {
  stage: InstallTaskStage
  code: 'DOWNLOAD_FAILED' | 'DEPLOY_FAILED'
  message: string
  technicalDetail: string
}

export interface InstallResult {
  installed: boolean
  installedVersion: string
  targetNode: InstallTargetNode
  autoStart: 'running' | 'failed' | 'skipped'
}

export interface InstallLaunchRequest {
  identifier: string
  referrer?: string
  targetNode?: InstallTargetNode
  offline: boolean
  installParams?: Record<string, unknown>
}

export interface InstallTask {
  taskId: string
  app: InstallAppInfo
  plan: InstallPlan
  stage: InstallTaskStage
  status: InstallTaskStatus
  progress: number | null
  summary: string
  currentResource?: string
  history: InstallTaskHistoryItem[]
  launchRequest?: InstallLaunchRequest
  failure?: InstallFailure
  result?: InstallResult
}
