export const CONTROL_PANEL_SERVICE_ID = 'control-panel'

export function buildAuthLoginTargetParams(redirectUrl: string) {
  if (redirectUrl) {
    return { redirect_url: redirectUrl }
  }
  return {
    target: {
      kind: 'system' as const,
      service_id: CONTROL_PANEL_SERVICE_ID,
    },
  }
}
