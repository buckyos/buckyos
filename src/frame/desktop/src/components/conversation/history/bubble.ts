import type { CSSProperties } from 'react'
import type { DID } from 'buckyos'

export interface MessageRenderContext {
  isGroup: boolean
  selfDid: DID
  messageIndex: number
  continued: boolean
  peerMarkdown: boolean
}

export const bubbleWidthClass = 'max-w-[85%] md:max-w-[min(75%,660px)]'
export const bodyTextClass = 'text-[16px] leading-[1.5] md:text-[15px]'

export function bubbleStyle(isSelf: boolean, continued: boolean): CSSProperties {
  return {
    '--mh-link': isSelf ? 'var(--cp-message-self-link)' : 'var(--cp-accent)',
    background: isSelf ? 'var(--cp-message-self-bg)' : 'var(--cp-message-peer-bg)',
    color: isSelf ? 'var(--cp-message-self-text)' : 'var(--cp-text)',
    border: isSelf ? '1px solid transparent' : '1px solid var(--cp-message-peer-border)',
    borderRadius: isSelf
      ? `18px ${continued ? 6 : 18}px 6px 18px`
      : `${continued ? 6 : 18}px 18px 18px 6px`,
    padding: '8px 12px',
  } as CSSProperties
}

/** Media stands on the canvas without a bubble; the footer under it uses the canvas colours. */
export function framelessStyle(): CSSProperties {
  return { '--mh-link': 'var(--cp-accent)', color: 'var(--cp-text)' } as CSSProperties
}

export function rowClass(isSelf: boolean, continued: boolean): string {
  return `flex ${isSelf ? 'justify-end' : 'justify-start'} ${continued ? 'pt-0.5' : 'pt-3'}`
}
