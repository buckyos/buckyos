/* ── Small UI primitives shared by the canvas app ── */

import clsx from 'clsx'
import { X } from 'lucide-react'
import { useEffect, useRef, useState, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode, type SelectHTMLAttributes, type TextareaHTMLAttributes } from 'react'
import { useCanvasEditor, useStoreState } from '../store/hooks'

export function Btn({
  variant = 'subtle',
  icon,
  active,
  className,
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: 'primary' | 'subtle' | 'ghost' | 'danger'; icon?: ReactNode; active?: boolean }) {
  return (
    <button type="button" className={clsx('aic-btn', `aic-btn-${variant}`, active && 'is-active', className)} {...rest}>
      {icon ? <span className="inline-flex [&>svg]:size-[14px]">{icon}</span> : null}
      {children}
    </button>
  )
}

export function IconBtn({
  icon,
  label,
  active,
  className,
  size = 28,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { icon: ReactNode; label: string; active?: boolean; size?: number }) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      className={clsx(
        'inline-flex items-center justify-center rounded-md text-[color:var(--cp-text)] transition hover:bg-[color:color-mix(in_srgb,var(--cp-text)_8%,transparent)] disabled:opacity-40 disabled:hover:bg-transparent [&>svg]:size-[15px]',
        active && 'bg-[color:color-mix(in_srgb,var(--cp-accent)_16%,transparent)] text-[color:var(--cp-accent)]',
        className,
      )}
      style={{ width: size, height: size }}
      {...rest}
    >
      {icon}
    </button>
  )
}

export function Input(props: InputHTMLAttributes<HTMLInputElement>) {
  return <input {...props} className={clsx('aic-input', props.className)} />
}

export function TextArea(props: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea {...props} className={clsx('aic-input resize-none leading-relaxed', props.className)} />
}

export function Select(props: SelectHTMLAttributes<HTMLSelectElement>) {
  return <select {...props} className={clsx('aic-input', props.className)} />
}

export function Field({ label, hint, children, inline }: { label: string; hint?: string; children: ReactNode; inline?: boolean }) {
  return (
    <label className={clsx('block', inline && 'flex items-center justify-between gap-3')}>
      <span className="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-[color:var(--cp-muted)]">{label}</span>
      {children}
      {hint ? <span className="mt-1 block text-[11px] text-[color:var(--cp-muted)]">{hint}</span> : null}
    </label>
  )
}

export function SectionTitle({ children, aside }: { children: ReactNode; aside?: ReactNode }) {
  return (
    <div className="mb-2 flex items-center justify-between">
      <span className="text-[11px] font-semibold uppercase tracking-[0.14em] text-[color:var(--cp-muted)]">{children}</span>
      {aside}
    </div>
  )
}

export type Tone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger' | 'ai'

const toneClass: Record<Tone, string> = {
  neutral: 'bg-[color:color-mix(in_srgb,var(--cp-text)_8%,transparent)] text-[color:var(--cp-muted)]',
  accent: 'bg-[color:color-mix(in_srgb,var(--cp-accent)_16%,transparent)] text-[color:var(--cp-accent)]',
  success: 'bg-[color:color-mix(in_srgb,var(--cp-success)_20%,transparent)] text-[color:color-mix(in_srgb,var(--cp-success)_70%,var(--cp-text))]',
  warning: 'bg-[color:color-mix(in_srgb,var(--cp-warning)_26%,transparent)] text-[color:color-mix(in_srgb,var(--cp-warning)_55%,var(--cp-text))]',
  danger: 'bg-[color:color-mix(in_srgb,var(--cp-danger)_16%,transparent)] text-[color:var(--cp-danger)]',
  ai: 'bg-[color:var(--aic-ai-soft)] text-[color:var(--aic-ai)]',
}

export function Badge({ tone = 'neutral', children, icon, className, title }: { tone?: Tone; children: ReactNode; icon?: ReactNode; className?: string; title?: string }) {
  return (
    <span title={title} className={clsx('inline-flex items-center gap-1 rounded-full px-2 py-[2px] text-[11px] font-medium leading-4 [&>svg]:size-[11px]', toneClass[tone], className)}>
      {icon}
      {children}
    </span>
  )
}

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="rounded border border-[color:var(--cp-border-opaque)] bg-[color:var(--cp-surface-2-opaque)] px-1.5 py-[1px] font-mono text-[10px]">{children}</kbd>
}

export function Modal({ open, title, onClose, children, footer, width = 560 }: { open: boolean; title: string; onClose: () => void; children: ReactNode; footer?: ReactNode; width?: number }) {
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation()
        onClose()
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [open, onClose])
  if (!open) return null
  return (
    <div className="absolute inset-0 z-[80] flex items-center justify-center bg-[color:color-mix(in_srgb,var(--cp-shadow)_35%,transparent)] p-6" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="aic-fade-in flex max-h-full w-full flex-col overflow-hidden rounded-xl border border-[color:var(--cp-border-opaque)] bg-[color:var(--cp-surface-opaque)] shadow-[var(--cp-window-shadow)]" style={{ maxWidth: width }} role="dialog" aria-label={title}>
        <div className="flex items-center justify-between border-b border-[color:var(--cp-border)] px-4 py-3">
          <span className="font-display text-sm font-semibold">{title}</span>
          <IconBtn icon={<X />} label="关闭" onClick={onClose} />
        </div>
        <div className="aic-scroll min-h-0 flex-1 overflow-y-auto px-4 py-4">{children}</div>
        {footer ? <div className="flex items-center justify-end gap-2 border-t border-[color:var(--cp-border)] px-4 py-3">{footer}</div> : null}
      </div>
    </div>
  )
}

export interface MenuItem {
  label: string
  onClick: () => void
  icon?: ReactNode
  danger?: boolean
  disabled?: boolean
  divider?: boolean
}

export function Menu({ items, at, onClose }: { items: MenuItem[]; at: { x: number; y: number }; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose()
    }
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose()
    window.addEventListener('mousedown', onDown, true)
    window.addEventListener('keydown', onKey, true)
    return () => {
      window.removeEventListener('mousedown', onDown, true)
      window.removeEventListener('keydown', onKey, true)
    }
  }, [onClose])
  return (
    <div ref={ref} className="aic-menu aic-fade-in" style={{ left: at.x, top: at.y }} role="menu">
      {items.map((it, i) =>
        it.divider ? (
          <div key={i} className="my-1 border-t border-[color:var(--cp-border)]" />
        ) : (
          <button
            key={i}
            type="button"
            role="menuitem"
            disabled={it.disabled}
            className={clsx(it.danger && 'is-danger')}
            onClick={() => {
              onClose()
              it.onClick()
            }}
          >
            {it.icon ? <span className="inline-flex [&>svg]:size-[13px]">{it.icon}</span> : null}
            {it.label}
          </button>
        ),
      )}
    </div>
  )
}

/** Button that opens a dropdown Menu below itself. */
export function MenuButton({ items, children, icon, variant = 'subtle' }: { items: MenuItem[]; children?: ReactNode; icon?: ReactNode; variant?: 'primary' | 'subtle' | 'ghost' }) {
  const [open, setOpen] = useState(false)
  const btn = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ x: 0, y: 0 })
  return (
    <div ref={btn} className="relative inline-flex">
      {children ? (
        <Btn
          variant={variant}
          icon={icon}
          onClick={() => {
            setPos({ x: 0, y: (btn.current?.offsetHeight ?? 28) + 4 })
            setOpen((v) => !v)
          }}
        >
          {children}
        </Btn>
      ) : (
        <IconBtn
          icon={icon}
          label="更多"
          onClick={() => {
            setPos({ x: 0, y: (btn.current?.offsetHeight ?? 28) + 4 })
            setOpen((v) => !v)
          }}
        />
      )}
      {open ? <Menu items={items} at={pos} onClose={() => setOpen(false)} /> : null}
    </div>
  )
}

export function EmptyState({ title, body, action }: { title: string; body?: string; action?: ReactNode }) {
  return (
    <div className="rounded-lg border border-dashed border-[color:var(--cp-border-opaque)] px-3 py-4 text-center">
      <p className="text-xs font-semibold">{title}</p>
      {body ? <p className="mt-1 text-[11px] leading-5 text-[color:var(--cp-muted)]">{body}</p> : null}
      {action ? <div className="mt-2 flex justify-center">{action}</div> : null}
    </div>
  )
}

export function ToastHost() {
  const { ui } = useStoreState()
  const { store } = useCanvasEditor()
  if (!ui.toast) return null
  const tone = ui.toast.tone === 'error' ? 'danger' : ui.toast.tone === 'success' ? 'success' : 'accent'
  return (
    <div className="pointer-events-none absolute inset-x-0 top-14 z-[90] flex justify-center">
      <div className={clsx('aic-fade-in pointer-events-auto flex max-w-[70%] items-center gap-3 rounded-lg border border-[color:var(--cp-border-opaque)] bg-[color:var(--cp-surface-opaque)] px-3 py-2 text-xs shadow-[var(--cp-panel-shadow)]')}>
        <Badge tone={tone}>{ui.toast.tone === 'error' ? '错误' : ui.toast.tone === 'success' ? '完成' : '提示'}</Badge>
        <span className="leading-5">{ui.toast.text}</span>
        <IconBtn icon={<X />} label="关闭" size={22} onClick={() => store.setUi({ toast: null })} />
      </div>
    </div>
  )
}

