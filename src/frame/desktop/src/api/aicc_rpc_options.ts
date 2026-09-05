export function toAiccRpcCallOptions(sessionToken: string | null): { sessionToken: string } | undefined {
  return sessionToken ? { sessionToken } : undefined
}
