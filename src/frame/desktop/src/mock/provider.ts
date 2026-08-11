import type { DesktopPayload, FormFactor, MockScenario } from '../models/ui'
import { buildDesktopPayload } from './data'

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

  return structuredClone(buildDesktopPayload(formFactor, scenario))
}
