/* ── Security & account section ── */

import { Shield, Key, ShieldCheck } from 'lucide-react'
import { Button, Switch, FormControlLabel } from '@mui/material'

interface SecuritySectionProps {
  twoFactorEnabled: boolean
  lastLogin: string
}

export function SecuritySection({ twoFactorEnabled, lastLogin }: SecuritySectionProps) {
  return (
    <div
      className="rounded-[22px] px-5 py-4"
      style={{
        background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
      }}
    >
      <div className="flex items-center gap-2 mb-3">
        <Shield size={16} style={{ color: 'var(--cp-accent)' }} />
        <h3
          className="font-display text-sm font-semibold"
          style={{ color: 'var(--cp-text)' }}
        >
          Security & Account
        </h3>
      </div>

      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Key size={14} style={{ color: 'var(--cp-muted)' }} />
            <span className="text-sm" style={{ color: 'var(--cp-text)' }}>Password</span>
          </div>
          <Button size="small" variant="outlined">
            Change
          </Button>
        </div>

        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <ShieldCheck size={14} style={{ color: 'var(--cp-muted)' }} />
            <span className="text-sm" style={{ color: 'var(--cp-text)' }}>Two-Factor Auth</span>
          </div>
          <FormControlLabel
            control={<Switch checked={twoFactorEnabled} size="small" />}
            label=""
          />
        </div>

        <div className="flex items-baseline gap-3 pt-1">
          <span className="text-[12px] font-medium w-24 shrink-0" style={{ color: 'var(--cp-muted)' }}>
            Last login
          </span>
          <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
            {new Date(lastLogin).toLocaleString()}
          </span>
        </div>
      </div>
    </div>
  )
}
