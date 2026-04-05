const MOCK_FLAG_VALUES = new Set(['1', 'true', 'yes', 'mock'])

const readMockFlag = () => {
  const raw = String(import.meta.env.VITE_CP_USE_MOCK ?? '').trim().toLowerCase()
  return MOCK_FLAG_VALUES.has(raw)
}

export const DESKTOP_USE_MOCK = readMockFlag()

export const isMockRuntime = () => DESKTOP_USE_MOCK

export const waitForMockLatency = async (ms = 80) => {
  if (ms <= 0) {
    return
  }
  await new Promise((resolve) => window.setTimeout(resolve, ms))
}
