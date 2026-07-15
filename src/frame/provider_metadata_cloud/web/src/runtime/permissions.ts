import type { ServiceRole } from '../datamodel/types'

export function canEditServiceRole(role: ServiceRole) {
  return role === 'tech' || role === 'ops'
}

export function canPublishServiceRole(role: ServiceRole) {
  return role === 'tech' || role === 'ops'
}
