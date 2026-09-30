import { createContext, useCallback, useContext } from 'react'
import { openPreview } from '../../../preview/launch'
import type { ConversationMessageReader } from '../history/types'
import { installMessageHubPreviewSources, type AttachmentKind, type MessageAttachment } from './source'

export interface MediaOpenRequest {
  attachment: MessageAttachment
  kind: AttachmentKind | null
  reader?: ConversationMessageReader
  hostContext?: string
}

export interface MessageMediaHostValue {
  open(request: MediaOpenRequest): void
  openSettings(): void
}

export interface ConversationMediaScopeValue {
  reader: ConversationMessageReader
  hostContext?: string
}

export const MessageMediaHostContext = createContext<MessageMediaHostValue | null>(null)
export const ConversationMediaScopeContext = createContext<ConversationMediaScopeValue | null>(null)

export function useMessageMediaHost(): MessageMediaHostValue | null {
  return useContext(MessageMediaHostContext)
}

export function useOpenAttachment(): (attachment: MessageAttachment, kind: AttachmentKind | null) => void {
  const host = useContext(MessageMediaHostContext)
  const scope = useContext(ConversationMediaScopeContext)
  return useCallback((attachment, kind) => {
    installMessageHubPreviewSources()
    if (host) {
      host.open({ attachment, kind, reader: scope?.reader, hostContext: scope?.hostContext })
      return
    }
    openPreview({ source: attachment.source, origin: { app: 'messagehub', hostContext: scope?.hostContext } })
  }, [host, scope])
}
