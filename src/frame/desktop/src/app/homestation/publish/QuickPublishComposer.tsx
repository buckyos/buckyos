import { useState } from 'react'
import {
  Image,
  Mic,
  Send,
  Video,
  Hash,
} from 'lucide-react'

interface QuickPublishComposerProps {
  t: (key: string, fallback: string) => string
  onPublish: (text: string) => void
}

export function QuickPublishComposer({
  t,
  onPublish,
}: QuickPublishComposerProps) {
  const [text, setText] = useState('')

  const handleSubmit = () => {
    if (!text.trim()) return
    onPublish(text.trim())
    setText('')
  }

  return (
    <div className="p-4">
      <h2 className="mb-4 text-lg font-bold" style={{ color: 'var(--cp-text)' }}>
        {t('homestation.newPost', 'New Post')}
      </h2>

      <div
        className="rounded-2xl p-3"
        style={{ background: 'color-mix(in srgb, var(--cp-text) 5%, transparent)' }}
      >
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={t('homestation.whatOnYourMind', "What's on your mind?")}
          className="w-full resize-none bg-transparent text-sm outline-none"
          style={{ color: 'var(--cp-text)', minHeight: 120 }}
          rows={5}
          autoFocus
        />

        {/* Character count */}
        <div className="flex items-center justify-between pt-2" style={{ borderTop: '1px solid var(--cp-border)' }}>
          <div className="flex items-center gap-2">
            <button
              type="button"
              className="flex h-9 w-9 items-center justify-center rounded-xl transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-text)_8%,transparent)]"
              style={{ color: 'var(--cp-muted)' }}
              title={t('homestation.addImage', 'Add image')}
            >
              <Image size={18} />
            </button>
            <button
              type="button"
              className="flex h-9 w-9 items-center justify-center rounded-xl transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-text)_8%,transparent)]"
              style={{ color: 'var(--cp-muted)' }}
              title={t('homestation.addVideo', 'Add video')}
            >
              <Video size={18} />
            </button>
            <button
              type="button"
              className="flex h-9 w-9 items-center justify-center rounded-xl transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-text)_8%,transparent)]"
              style={{ color: 'var(--cp-muted)' }}
              title={t('homestation.addVoice', 'Record voice')}
            >
              <Mic size={18} />
            </button>
            <button
              type="button"
              className="flex h-9 w-9 items-center justify-center rounded-xl transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-text)_8%,transparent)]"
              style={{ color: 'var(--cp-muted)' }}
              title={t('homestation.addTopic', 'Add topic')}
            >
              <Hash size={18} />
            </button>
          </div>

          <div className="flex items-center gap-3">
            <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              {text.length}
            </span>
            <button
              type="button"
              onClick={handleSubmit}
              disabled={!text.trim()}
              className="flex items-center gap-1.5 rounded-xl px-4 py-2 text-sm font-medium transition-opacity disabled:opacity-40"
              style={{ background: 'var(--cp-accent)', color: 'white' }}
            >
              <Send size={14} />
              {t('homestation.publish', 'Publish')}
            </button>
          </div>
        </div>
      </div>

      <p className="mt-3 text-xs" style={{ color: 'var(--cp-muted)' }}>
        {t('homestation.publishHint', 'Your post will appear on your personal feed and be visible to your followers.')}
      </p>
    </div>
  )
}
