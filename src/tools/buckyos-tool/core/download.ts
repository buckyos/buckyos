import { isAbsolute, resolve } from 'node:path'
import type { CommandContext } from './context.ts'
import { resolveServiceUrl } from './runtime.ts'
import { ToolError, UsageError } from './errors.ts'

export interface ArtifactDownload {
  path: string
  size: number
  sha256: string
}

export type ArtifactFetcher = (
  url: URL,
  ctx: CommandContext,
) => Promise<Uint8Array>

export async function downloadArtifact(
  ctx: CommandContext,
  service: string,
  rawUrl: string,
  rawPath: string,
  fetcher: ArtifactFetcher = defaultArtifactFetcher,
): Promise<ArtifactDownload> {
  const path = isAbsolute(rawPath) ? rawPath : resolve(ctx.cwd, rawPath)
  const base = new URL(resolveServiceUrl(ctx.config, service))
  const url = new URL(rawUrl, `${base.protocol}//${base.host}`)
  if (url.origin !== base.origin || url.username || url.password) {
    throw new ToolError('INVALID_DOWNLOAD_URL', 'artifact URL must use the configured Zone origin')
  }
  const bytes = await fetcher(url, ctx)
  try {
    const file = await Deno.open(path, { createNew: true, write: true })
    try {
      let offset = 0
      while (offset < bytes.length) offset += await file.write(bytes.subarray(offset))
      await file.sync()
    } finally {
      file.close()
    }
  } catch (error) {
    if (error instanceof Deno.errors.AlreadyExists) {
      throw new UsageError('OUTPUT_EXISTS', `output path already exists: ${path}`)
    }
    try {
      await Deno.remove(path)
    } catch {}
    throw error
  }
  const digest = await crypto.subtle.digest('SHA-256', Uint8Array.from(bytes).buffer)
  return { path, size: bytes.byteLength, sha256: toHex(new Uint8Array(digest)) }
}

async function defaultArtifactFetcher(url: URL, ctx: CommandContext): Promise<Uint8Array> {
  const token = ctx.session ? (await ctx.session.ensureValid()).token : undefined
  const response = await fetch(url, {
    headers: token ? { authorization: `Bearer ${token}` } : undefined,
    signal: ctx.signal,
  })
  if (!response.ok) {
    throw new ToolError(
      'DOWNLOAD_FAILED',
      `artifact download failed with HTTP ${response.status}`,
    )
  }
  return new Uint8Array(await response.arrayBuffer())
}

function toHex(bytes: Uint8Array): string {
  return [...bytes].map((byte) => byte.toString(16).padStart(2, '0')).join('')
}
