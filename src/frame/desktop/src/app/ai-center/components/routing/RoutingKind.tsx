import type { ReactNode } from 'react'
import { useI18n } from '../../../../i18n/provider'
import { KIND_STYLE, lastSegment, type DirectoryKind, type RoutingCommand } from '../../datamodel/routing'

export function KindIcon({ kind, size = 15 }: { kind: DirectoryKind; size?: number }) {
  const { color, icon: Icon } = KIND_STYLE[kind]
  return (
    <span
      className="inline-flex shrink-0 items-center justify-center rounded-md"
      style={{ width: size + 11, height: size + 11, color, background: `color-mix(in srgb, ${color} 14%, transparent)` }}
      aria-hidden
    >
      <Icon size={size} />
    </span>
  )
}

export function CommandLabel({ command, onOpen }: { command: RoutingCommand; onOpen?: (path: string) => void }): ReactNode {
  const { t } = useI18n()
  switch (command.kind) {
    case 'vendor_factor':
      return <span className="block">{t('aiCenter.routing.command.vendor', 'Vendor {{vendor}}', { vendor: command.vendor })}</span>
    case 'spec_factor':
      return <span className="block">{t('aiCenter.routing.command.spec', 'Specification {{spec}}', { spec: lastSegment(command.spec) })}</span>
    case 'model_factor':
      return <span className="block">{t('aiCenter.routing.command.model', 'Model {{vendor}}/{{model}}', { vendor: command.vendor, model: command.model })}</span>
    case 'item_weight':
      return (
        <span className="block break-all">
          {t('aiCenter.routing.command.item', 'Item {{item}} in', { item: command.item })}{' '}
          {onOpen ? (
            <button type="button" onClick={() => onOpen(command.path)} className="font-mono hover:underline" style={{ color: 'var(--cp-accent)' }}>{command.path}</button>
          ) : <span className="font-mono">{command.path}</span>}
        </span>
      )
  }
}

