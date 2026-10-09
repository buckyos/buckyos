import type { DesktopPayload, FormFactor, MockScenario } from '../models/ui'
import { buildDesktopPayload } from './data'
import { contentFixtureApps, contentFixtureEnabled } from './content'

interface ProviderArgs {
  formFactor: FormFactor
  scenario: MockScenario
}

function delay(ms: number) {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms)
  })
}

export async function fetchDesktopPayload({
  formFactor,
  scenario,
}: ProviderArgs): Promise<DesktopPayload> {
  await delay(scenario === 'normal' ? 360 : 420)

  if (scenario === 'error') {
    throw new Error('mock.provider.desktop_unavailable')
  }

  const payload = structuredClone(buildDesktopPayload(formFactor, scenario))
  if (contentFixtureEnabled()) payload.apps.push(...contentFixtureApps(payload.apps.find(app => app.id === 'files')!))
  return payload
}
