/* ── Binding management section ── */

import { Link2, Plus, AlertCircle, Check, Clock } from 'lucide-react'
import { Button, IconButton } from '@mui/material'
import type { MessageTunnelBinding } from '../../mock/types'

interface BindingsSectionProps {
  bindings: MessageTunnelBinding[]
}

const statusIcon = {
  active: Check,
  pending: Clock,
  error: AlertCircle,
}

const statusColor = {
  active: 'var(--cp-success)',
  pending: 'var(--cp-warning)',
  error: 'var(--cp-danger)',
}

export function BindingsSection({ bindings }: BindingsSectionProps) {
  return (
    <div
      className="rounded-[22px] px-5 py-4"
      style={{
        background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
      }}
    >
      <div className="flex items-center justify-between mb-3">
        <div className="flex items-center gap-2">
          <Link2 size={16} style={{ color: 'var(--cp-accent)' }} />
          <h3
            className="font-display text-sm font-semibold"
            style={{ color: 'var(--cp-text)' }}
          >
            Message Tunnel Bindings
          </h3>
        </div>
        <Button size="small" startIcon={<Plus size={14} />} variant="text">
          Add
        </Button>
      </div>

      {bindings.length === 0 ? (
        <div className="text-sm py-3" style={{ color: 'var(--cp-muted)' }}>
          No bindings configured. Add a binding to connect external messaging channels.
        </div>
      ) : (
        <div className="space-y-2">
          {bindings.map((b) => {
            const StatusIcon = statusIcon[b.status]
            const color = statusColor[b.status]
            return (
              <div
                key={b.id}
                className="flex items-center gap-3 px-3 py-2.5 rounded-[14px]"
                style={{
                  background: 'color-mix(in srgb, var(--cp-surface) 80%, transparent)',
                  border: '1px solid color-mix(in srgb, var(--cp-border) 40%, transparent)',
                }}
              >
                <div
                  className="shrink-0 flex items-center justify-center rounded-full"
                  style={{
                    width: 28,
                    height: 28,
                    background: `color-mix(in srgb, ${color} 14%, var(--cp-surface))`,
                    color,
                  }}
                >
                  <StatusIcon size={14} />
                </div>

                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium capitalize" style={{ color: 'var(--cp-text)' }}>
                    {b.platform}
                  </div>
                  <div className="text-[11px] truncate" style={{ color: 'var(--cp-muted)' }}>
                    {b.displayId}
                  </div>
                </div>

                {b.lastSyncAt && (
                  <span className="text-[10px] shrink-0" style={{ color: 'var(--cp-muted)' }}>
                    {new Date(b.lastSyncAt).toLocaleDateString()}
                  </span>
                )}

                <IconButton size="small" aria-label="Remove binding">
                  <Link2 size={12} />
                </IconButton>
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}
