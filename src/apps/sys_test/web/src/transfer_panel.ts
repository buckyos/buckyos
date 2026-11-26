/**
 * A minimal `transfer.v1` participant (剪贴板规范 §5, §8): offers a text to other Apps through the desktop Shell's
 * "Drag to another app" handle and lists what other Apps delivered here, so delivery between two App subdomains
 * (iframe ↔ iframe, system App ↔ iframe) can be checked by hand.
 */
import type { AppFrameClient, AppFrameTransferHandlers, TransferDeliveryResult } from 'buckyos/app-frame'
import { bindText, localize, t } from './i18n'

const CONTENT_REFS = 'application/vnd.buckyos.content-refs+json'
let client: AppFrameClient | undefined
let offer: { offerId: string; text: string } | undefined

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T | null

export function renderTransferPanel(): void {
  const card = document.createElement('section')
  card.className = 'group'
  card.dataset.group = 'transfer'
  card.innerHTML = `
    <div class="group-header"><h2 class="group-title" data-i18n="transfer.title">Content between Apps</h2><span class="group-id">transfer.v1</span></div>
    <p class="group-desc" data-i18n="transfer.intro">Offer this text to other Apps, then use “Drag to another app” in this window’s title bar. Content other Apps deliver here is listed below.</p>
    <div class="group-actions">
      <textarea id="transfer-text" rows="2" data-i18n-label="transfer.text" aria-label="Text to offer" data-i18n-default="transfer.defaultText">Hello from sys_test</textarea>
      <button id="transfer-offer" class="btn primary" data-testid="transfer-offer" data-i18n="transfer.offer" disabled>Offer to other Apps</button>
      <span id="transfer-status" class="auth-status" role="status" data-i18n="transfer.unavailable">Open this App in the desktop to exchange content</span>
    </div>
    <ul id="transfer-received" aria-label="Received" data-i18n-label="transfer.received"></ul>`
  document.getElementById('groups')?.after(card)
  localize(card)
  $('transfer-offer')?.addEventListener('click', () => { void offerText() })
}

async function offerText(): Promise<void> {
  if (!client) return
  const text = $<HTMLTextAreaElement>('transfer-text')?.value ?? ''
  if (offer) client.withdrawTransfer(offer.offerId)
  offer = undefined
  if (!text) return
  const reply = await client.offerTransfer({ purpose: 'drag', items: [{ kind: 'text', formats: ['text/plain'], operations: ['copy'], ...(text.length <= 65536 ? { inline: { 'text/plain': text } } : {}) }] })
  if (reply.ok) offer = { offerId: reply.offerId, text }
  const status = $('transfer-status')
  if (status) bindText(status, reply.ok ? 'transfer.offered' : 'transfer.refused', reply.ok ? undefined : { code: reply.code })
}

function received(format: string, value: unknown): string {
  if (format === CONTENT_REFS && value && typeof value === 'object') {
    const source = (value as { source?: { path?: string; objectId?: string } }).source
    return source?.path ?? source?.objectId ?? ''
  }
  return typeof value === 'string' ? value : value instanceof File ? `${value.name} (${value.size} B)` : ''
}

/** The handlers this page gives its App Frame client: takes text and Zone references anywhere, offers its text. */
export function transferHandlers(get: () => AppFrameClient | undefined): AppFrameTransferHandlers {
  return {
    onAvailable: available => {
      client = get()
      const button = $<HTMLButtonElement>('transfer-offer')
      if (button) button.disabled = !available
      const status = $('transfer-status')
      if (status) bindText(status, available ? 'transfer.available' : 'transfer.unavailable')
    },
    onProbe: probe => {
      const accept = probe.items.flatMap(item => {
        const format = [CONTENT_REFS, 'text/plain', 'files'].find(f => item.formats.includes(f))
        return format ? [{ itemKey: item.itemKey, format, operation: item.operations.includes('link') && format === CONTENT_REFS ? 'link' as const : 'copy' as const }] : []
      })
      return accept.length ? { targetId: 'received', action: t('transfer.action'), accept } : null
    },
    onDeliver: (delivery): TransferDeliveryResult => {
      const list = $('transfer-received')
      for (const item of delivery.items) {
        const row = document.createElement('li')
        row.dataset.format = item.format
        row.textContent = `${item.format}: ${received(item.format, item.value)}`
        list?.append(row)
      }
      return { status: 'done', items: delivery.items.map(item => ({ itemKey: item.itemKey, status: 'done' as const })) }
    },
    onRead: read => read.items.map(item => offer && read.offerId === offer.offerId && item.format === 'text/plain'
      ? { ...item, value: offer.text }
      : { ...item, error: 'stale' as const }),
  }
}
