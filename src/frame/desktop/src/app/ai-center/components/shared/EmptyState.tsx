import type { ReactNode } from 'react'

interface EmptyStateProps {
  icon: ReactNode
  title: string
  description?: string
  action?: { label: string; onClick: () => void }
}

export function EmptyState({ icon, title, description, action }: EmptyStateProps) {
  return (
    <div className="flex flex-col items-center justify-center py-16 px-4 text-center">
      <div className="mb-4" style={{ color: 'var(--cp-muted)' }}>
        {icon}
      </div>
      <h2
        className="text-xl font-semibold mb-2"
        style={{ color: 'var(--cp-text)' }}
      >
        {title}
      </h2>
      {description && (
        <p
          className="text-sm max-w-md mb-6"
          style={{ color: 'var(--cp-muted)' }}
        >
          {description}
        </p>
      )}
      {action && (
        <button
          type="button"
          onClick={action.onClick}
          className="px-5 py-2.5 rounded-lg text-sm font-medium transition-opacity hover:opacity-80"
          style={{
            background: 'var(--cp-accent)',
            color: '#fff',
          }}
        >
          {action.label}
        </button>
      )}
    </div>
  )
}
