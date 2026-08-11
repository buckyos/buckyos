import { buckyos } from 'buckyos'

export const callRpc = async <T>(
  method: string,
  params: Record<string, unknown> = {},
): Promise<{ data: T | null; error: unknown }> => {
  try {
    const rpcClient = buckyos.getServiceRpcClient('control-panel')

    const result = await rpcClient.call(method, params)
    if (!result || typeof result !== 'object') {
      throw new Error(`Invalid ${method} response`)
    }
    return { data: result as T, error: null }
  } catch (error) {
    console.error(error)
    return { data: null, error }
  }
}
