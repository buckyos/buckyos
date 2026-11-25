import { buckyos } from 'buckyos'

export type RpcResult<T> = { data: T | null; error: unknown }

/** A shared call as the pages take it: `{ data, error }` instead of a rejection. */
export const settle = async <T>(call: Promise<T>): Promise<RpcResult<T>> => {
  try {
    return { data: await call, error: null }
  } catch (error) {
    console.error(error)
    return { data: null, error }
  }
}

export interface RpcCallOptions {
  sessionToken?: string | null
}

export const callRpc = async <T>(
  method: string,
  params: Record<string, unknown> = {},
  options: RpcCallOptions = {},
): Promise<{ data: T | null; error: unknown }> => {
  try {
    const rpcClient = buckyos.getServiceRpcClient('control-panel')

    const result = await rpcClient.call(method, params, options)
    if (!result || typeof result !== 'object') {
      throw new Error(`Invalid ${method} response`)
    }
    return { data: result as T, error: null }
  } catch (error) {
    console.error(error)
    return { data: null, error }
  }
}
