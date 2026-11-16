import { ThingConversation } from '../conversation/ThingConversation'

/** Conversation mode (APP7): the stage shows the browser conversation instead of the screen. */
export function ConversationPanel() {
  return <div className="ab-chat-panel" data-testid="ab-chat-panel"><ThingConversation /></div>
}
