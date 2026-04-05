/* ── App Service mock types ── */

export type AppServiceStatus = 'running' | 'starting' | 'stopped' | 'error' | 'installing'

export type ServiceLayer = 'app' | 'system' | 'kernel'

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
  installProgress?: number
}

export type InstallSource = 'url' | 'object-id' | 'file'

export interface InstallPermission {
  label: string
  description: string
}

export interface InstallAppInfo {
  name: string
  version: string
  description: string
  iconKey: string
  permissions: InstallPermission[]
}
