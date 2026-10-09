import { ContentRegistryClient, emptyDefaults, type ContentRegistry } from 'buckyos/content'
import type { AppDefinition } from '../models/ui'

export const contentFixtureEnabled = () => new URLSearchParams(location.search).has('contentApps')
export const contentFixtureApps = (base: AppDefinition): AppDefinition[] => ['notes', 'reader'].map(name => ({
  ...base, id: `${name}-fixture`, appInstanceId: `${name}.fixture.bns.did@owner`,
  labelKey: `${name === 'notes' ? 'Notebook' : 'Reader'} Fixture`, tier: 'external', webHosts: [`${name}-fixture`],
}))
const registry: ContentRegistry = { schema_version: 1, handlers: Object.fromEntries(['notes', 'reader'].map(name => {
  const instance = `${name}.fixture.bns.did@owner`
  return [`${instance}#text`, { provider: 'app', app_instance_id: instance, handler_id: 'text', handler_version: 1,
    enabled: true, selectors: [{ mime: 'text/markdown' }], intents: { open: {
      entry: { type: 'web', path: '/open?src={source}&session={session}' }, modes: ['edit'], window: name === 'notes' ? 'reuse' : 'new', priority: name === 'notes' ? 60 : 40,
    } } }]
})) }
export const mockContentRegistry = new ContentRegistryClient({
  get: async key => ({ value: key === 'system/content_registry' ? JSON.stringify(registry) : localStorage.getItem('mock.content.defaults') ?? JSON.stringify(emptyDefaults()) }),
  set: async (_key, value) => { localStorage.setItem('mock.content.defaults', value) },
}, 'owner')
