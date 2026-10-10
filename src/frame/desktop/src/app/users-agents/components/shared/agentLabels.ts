import type { AgentLifecycle } from '../../datamodel/types'

export function agentStatusLabelKey(status: AgentLifecycle): string {
  return `usersAgents.agentStatus.${status}`
}

export function agentStatusTone(status: AgentLifecycle): 'success' | 'warning' | 'danger' | 'muted' {
  switch (status) {
    case 'running':
      return 'success'
    case 'creating':
      return 'warning'
    case 'failed':
      return 'danger'
    default:
      return 'muted'
  }
}
