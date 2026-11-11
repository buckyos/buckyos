import type { AppDeploymentIdentity, AppDeploymentObservation, AppInstallationStatus, AppSummary } from '../../api/app_mgr'
import { createBackendAppDefinitionMapper, type BackendAppMapperOptions } from '../backend-apps'
import { pickLocalized } from '../../presentation/model'
import { emptyRuntime } from './mock/fixtures'
import type { AppServiceItem, AppServiceStatus, RuntimeType, RuntimeView } from './types'

export function sameDeployment(left: AppDeploymentIdentity | null | undefined, right: AppDeploymentIdentity) {
  return Boolean(left && left.app_instance_id === right.app_instance_id
    && left.task_id === right.task_id && left.app_doc_object_id === right.app_doc_object_id
    && left.spec_generation === right.spec_generation && left.pikg_digest === right.pikg_digest)
}

function runtimeType(summary: AppSummary, snapshot?: AppInstallationStatus | null): RuntimeType {
  if (summary.runtime_type === 'web' || summary.runtime_type === 'agent') return summary.runtime_type
  const spec = snapshot?.desired_spec
  const packages = spec?.packages?.length
    ? spec.packages.map(pkg => pkg.sub_pkg_name)
    : Object.keys(spec?.app_doc.pkg_list ?? {})
  const script = packages.includes('script')
  const docker = packages.includes('docker')
  return script !== docker ? (script ? 'script' : 'docker') : 'unknown'
}

function runtimeFromStatus(summary: AppSummary, snapshot?: AppInstallationStatus | null, now = Date.now()): RuntimeView {
  const runtime = emptyRuntime(runtimeType(summary, snapshot), 'unknown')
  runtime.agent_bindings = null
  if (!snapshot || snapshot.app_instance_id !== summary.app_instance_id
    || snapshot.desired_deployment.app_instance_id !== summary.app_instance_id) return runtime
  runtime.expected_deployment = snapshot.desired_deployment
  const fresh = (e: AppDeploymentObservation) => sameDeployment(e.deployment, snapshot.desired_deployment)
    && e.observed_at * 1000 <= now && e.expires_at * 1000 > now && e.expires_at > e.observed_at
  const reports = (snapshot.runtime_instances ?? []).filter(fresh)
  const web = (snapshot.static_web_evidence ?? []).filter(fresh)
  const observations = runtime.type === 'web' ? web : reports
  const ready = runtime.type === 'web'
    ? web.filter(e => !e.deployment_error && e.materialized_at > 0
      && e.gateway_config_generation && e.gateway_ready_at >= e.materialized_at
      && e.gateway_ready_at <= e.observed_at)
    : reports.filter(e => e.state === 'started' && e.health === 'healthy' && !e.deployment_error)
  const required = Math.max(1, snapshot.desired_instance_count)
  const stopped = !snapshot.desired_spec.enable || snapshot.desired_spec.state === 'stopped'
  let status: AppServiceStatus = 'unknown'
  if (!stopped && ready.length >= required) status = 'running'
  else if (observations.some(e => e.deployment_error) || reports.some(e => e.health === 'unhealthy')) status = 'error'
  else if (stopped && reports.length >= required && reports.every(e => ['stopped', 'notexist', 'exited'].includes(e.state))) status = 'stopped'
  else if (stopped && reports.some(e => e.state === 'started')) status = 'stopping'
  else if (reports.some(e => e.state === 'deploying')) status = 'deploying'
  else if (reports.some(e => e.state === 'started')) status = 'starting'
  runtime.status = status
  const evidence = status === 'running' ? ready.slice(0, required) : observations
  if (evidence.length) {
    runtime.evidence = {
      deployment: snapshot.desired_deployment,
      observed_at: Math.max(...evidence.map(e => e.observed_at)) * 1000,
      valid_until: Math.min(...evidence.map(e => e.expires_at)) * 1000,
    }
  }
  const health = status === 'running' ? 'READY' : observations.length ? 'NOT_READY' : 'UNKNOWN'
  if (runtime.type === 'web') runtime.web_accessible = health
  else if (runtime.type === 'agent') runtime.environment_ready = health
  else runtime.process_health = health
  runtime.reason = observations.find(e => e.deployment_error)?.deployment_error?.code ?? null
  return runtime
}

export function appServiceItemFromSummary(summary: AppSummary, options: BackendAppMapperOptions = {}, snapshot?: AppInstallationStatus | null, now = Date.now()): AppServiceItem {
  const definition = createBackendAppDefinitionMapper([], options)(summary)
  const runtime = runtimeFromStatus(summary, snapshot, now)
  const address = definition.webHosts?.[0] && options.zoneHost
    ? `${options.protocol ?? 'https:'}//${definition.webHosts[0]}.${options.zoneHost}`
    : undefined
  return {
    id: summary.app_instance_id,
    app_instance_id: summary.app_instance_id,
    system_service_id: null,
    owner_user_id: summary.owner_user_id,
    app_did: definition.did ?? null,
    name: pickLocalized(definition.text?.title, 'en') ?? summary.app_id,
    description: '',
    iconKey: summary.app_id,
    ...(definition.iconUrl ? { iconUrl: definition.iconUrl } : {}),
    ...(definition.text ? { text: definition.text } : {}),
    version: summary.version,
    layer: 'app',
    status: runtime.status,
    runtime,
    record: null,
    docker: null,
    diagnostics: [],
    spec: {
      enable: String(summary.enable),
      expected_instance_count: String(summary.expected_instance_count),
    },
    settings: {},
    serviceInfo: address ? { endpoint: address } : {},
    logs: [],
    available_actions: [],
    operation: null,
    operation_error: null,
  }
}
