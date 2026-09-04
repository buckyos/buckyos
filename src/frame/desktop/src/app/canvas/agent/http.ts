/* ── HTTP / SSE adapter for a real BuckyOS Agent service (PRD FR-AGENT-006 / §13.3) ── */

import { AgentRunError, type AgentRunEvent, type AgentRunRequest, type AgentStage, type CanvasAgentAdapter, type CanvasPatch } from './contracts'

export interface HttpAgentConfig {
  baseUrl: string
  timeoutMs: number
}

export class HttpCanvasAgentAdapter implements CanvasAgentAdapter {
  id = 'http'
  private readonly getConfig: () => HttpAgentConfig
  constructor(getConfig: () => HttpAgentConfig) {
    this.getConfig = getConfig
  }

  private url(path: string): string {
    const base = this.getConfig().baseUrl.replace(/\/+$/, '')
    return `${base}${path}`
  }

  async health() {
    const cfg = this.getConfig()
    if (!cfg.baseUrl.trim()) return { available: false, message: '尚未配置 Agent 服务地址' }
    try {
      const ctrl = new AbortController()
      const t = setTimeout(() => ctrl.abort(), 4000)
      const res = await fetch(this.url('/api/agent/health'), { signal: ctrl.signal })
      clearTimeout(t)
      if (!res.ok) return { available: false, message: `服务返回 ${res.status}` }
      return { available: true, message: `已连接 ${cfg.baseUrl}` }
    } catch (e) {
      return { available: false, message: `无法连接：${e instanceof Error ? e.message : String(e)}` }
    }
  }

  async run(request: AgentRunRequest, onEvent: (e: AgentRunEvent) => void, signal: AbortSignal): Promise<CanvasPatch> {
    let jobId: string
    try {
      const res = await fetch(this.url('/api/agent/jobs'), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(request),
        signal,
      })
      if (!res.ok) throw new AgentRunError('unavailable', `Agent 服务拒绝任务（HTTP ${res.status}）`)
      const body = (await res.json()) as { jobId: string; status: string }
      jobId = body.jobId
    } catch (e) {
      if (e instanceof AgentRunError) throw e
      if (signal.aborted) throw new AgentRunError('cancelled', '已取消')
      throw new AgentRunError('unavailable', `无法连接 Agent 服务：${e instanceof Error ? e.message : String(e)}`)
    }

    const cancel = () => {
      fetch(this.url(`/api/agent/jobs/${jobId}/cancel`), { method: 'POST' }).catch(() => undefined)
    }
    signal.addEventListener('abort', cancel, { once: true })

    try {
      await this.consumeEvents(jobId, onEvent, signal)
      const res = await fetch(this.url(`/api/agent/jobs/${jobId}/result`), { signal })
      if (!res.ok) throw new AgentRunError('failed', `无法获取结果（HTTP ${res.status}）`)
      return (await res.json()) as CanvasPatch
    } catch (e) {
      if (e instanceof AgentRunError) throw e
      if (signal.aborted) throw new AgentRunError('cancelled', '已取消')
      throw new AgentRunError('failed', e instanceof Error ? e.message : String(e))
    } finally {
      signal.removeEventListener('abort', cancel)
    }
  }

  private async consumeEvents(jobId: string, onEvent: (e: AgentRunEvent) => void, signal: AbortSignal) {
    const res = await fetch(this.url(`/api/agent/jobs/${jobId}/events`), { signal, headers: { accept: 'text/event-stream' } })
    if (!res.ok || !res.body) throw new AgentRunError('failed', `事件流不可用（HTTP ${res.status}）`)
    const reader = res.body.getReader()
    const decoder = new TextDecoder()
    let buffer = ''
    let eventName = 'message'
    let data = ''
    const flush = () => {
      if (!data) return
      try {
        const payload = JSON.parse(data) as Record<string, unknown>
        if (eventName === 'status') onEvent({ type: 'status', stage: (payload.stage as AgentStage) ?? 'running', message: String(payload.message ?? '') })
        else if (eventName === 'progress') onEvent({ type: 'progress', stage: (payload.stage as 'running') ?? 'running', percent: Number(payload.percent ?? 0), message: String(payload.message ?? '') })
        else if (eventName === 'warning') onEvent({ type: 'warning', message: String(payload.message ?? '') })
        else if (eventName === 'completed') onEvent({ type: 'completed', jobId: String(payload.jobId ?? jobId) })
        else if (eventName === 'failed') throw new AgentRunError('failed', String(payload.message ?? 'Agent 运行失败'))
        else onEvent({ type: 'log', message: data })
      } catch (e) {
        if (e instanceof AgentRunError) throw e
        onEvent({ type: 'log', message: data })
      }
      eventName = 'message'
      data = ''
    }
    for (;;) {
      const { value, done } = await reader.read()
      if (done) break
      buffer += decoder.decode(value, { stream: true })
      let idx: number
      while ((idx = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, idx).replace(/\r$/, '')
        buffer = buffer.slice(idx + 1)
        if (line === '') flush()
        else if (line.startsWith('event:')) eventName = line.slice(6).trim()
        else if (line.startsWith('data:')) data += line.slice(5).trim()
      }
      if (eventName === 'completed' && data === '') return
    }
    flush()
  }
}
