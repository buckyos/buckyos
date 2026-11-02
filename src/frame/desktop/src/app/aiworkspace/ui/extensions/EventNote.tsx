/* The corner note of a Block whose control raised an event in design mode (扩展控件 P2, Q27): debugging information,
 * nothing ran. */

import { useSyncExternalStore } from 'react'
import { useAiwsText } from '../../useAiwsText'
import { designNote, designNotesVersion, subscribeDesignNotes } from './events'

export function EventNote({ cellId }: { cellId: string }) {
  const { t } = useAiwsText()
  useSyncExternalStore(subscribeDesignNotes, designNotesVersion)
  const note = designNote(cellId)
  if (!note) return null
  return <div className="aiws-event-note" role="status" data-testid={`aiws-event-note-${cellId}`} data-event={note.event}>{t('aiworkspace.ext.event.designNote', { name: note.event })}</div>
}
