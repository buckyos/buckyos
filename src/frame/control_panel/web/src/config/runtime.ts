const MOCK_FLAG_VALUES = new Set(['1', 'true', 'yes', 'mock'])

const readMockFlag = () => {
  const raw = String(import.meta.env.VITE_CP_USE_MOCK ?? '').trim().toLowerCase()
  return MOCK_FLAG_VALUES.has(raw)
}

const encodeBase64Url = (value: string) =>
  btoa(value)
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/g, '')

export const CONTROL_PANEL_USE_MOCK = readMockFlag()
export const CONTROL_PANEL_MOCK_USERNAME = 'mock.admin'
export const CONTROL_PANEL_MOCK_USER_ID = 'mock-admin-user'
export const CONTROL_PANEL_MOCK_USER_TYPE = 'owner'
export const CONTROL_PANEL_MOCK_REFRESH_TOKEN = 'mock-refresh-token'
export const CONTROL_PANEL_MOCK_SESSION_TOKEN = (() => {
  const header = encodeBase64Url(JSON.stringify({ alg: 'HS256', typ: 'JWT' }))
  const payload = encodeBase64Url(
    JSON.stringify({
      sub: CONTROL_PANEL_MOCK_USERNAME,
      uid: CONTROL_PANEL_MOCK_USER_ID,
      role: CONTROL_PANEL_MOCK_USER_TYPE,
      iss: 'control-panel-mock',
    }),
  )
  return `${header}.${payload}.mock-signature`
})()

export const isMockRuntime = () => CONTROL_PANEL_USE_MOCK

export const waitForMockLatency = async (ms = 80) => {
  if (ms <= 0) {
    return
  }
  await new Promise((resolve) => window.setTimeout(resolve, ms))
}
