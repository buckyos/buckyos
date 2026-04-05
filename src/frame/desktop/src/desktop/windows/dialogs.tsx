/* eslint-disable react-refresh/only-export-components */
import clsx from 'clsx'
import { X } from 'lucide-react'
import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import type { AppDefinition } from '../../models/ui'

export type WindowDialogSurface = 'desktop' | 'mobile'
export type WindowDialogPresentation = 'auto' | 'modal' | 'sheet' | 'fullscreen'
export type WindowDialogSize = 'sm' | 'md' | 'lg' | 'fullscreen'

export interface WindowDialogControls<TResult = unknown> {
  close: (result?: TResult) => void
  dismiss: () => void
}

export interface WindowDialogOptions<TResult = unknown> {
  title?: string
  description?: string
  presentation?: WindowDialogPresentation
  size?: WindowDialogSize
  dismissible?: boolean
  closeOnBackdrop?: boolean
  renderBody: (controls: WindowDialogControls<TResult>) => ReactNode
  renderActions?: (controls: WindowDialogControls<TResult>) => ReactNode
}

export interface WindowDialogApi {
  canPresent: (presentation: WindowDialogPresentation) => boolean
  close: (dialogId: string, result?: unknown) => void
  dismiss: (dialogId: string) => void
  dismissTop: () => void
  isOpen: boolean
  open: <TResult = unknown>(
    options: WindowDialogOptions<TResult>,
  ) => Promise<TResult | undefined>
  permissions: WindowDialogPermissions
  stackDepth: number
}

export interface WindowDialogPermissions {
  fullscreen: boolean
}

interface ActiveWindowDialog<TResult = unknown> {
  id: string
  options: WindowDialogOptions<TResult>
  resolve: (result: TResult | undefined) => void
}

const WindowDialogContext = createContext<WindowDialogApi | null>(null)

export class WindowDialogPermissionError extends Error {
  code = 'permission_denied' as const
  presentation: WindowDialogPresentation

  constructor(presentation: WindowDialogPresentation) {
    super(`Permission denied for dialog presentation: ${presentation}`)
    this.name = 'WindowDialogPermissionError'
    this.presentation = presentation
  }
}

function resolvePresentation(
  _surface: WindowDialogSurface,
  presentation: WindowDialogPresentation,
) {
  if (presentation !== 'auto') {
    return presentation
  }

  return 'modal'
}

function isPresentationAllowed(
  permissions: WindowDialogPermissions,
  presentation: WindowDialogPresentation,
) {
  if (presentation !== 'fullscreen') {
    return true
  }

  return permissions.fullscreen
}

function dialogSizeClass(size: WindowDialogSize, presentation: WindowDialogPresentation) {
  if (presentation === 'fullscreen' || size === 'fullscreen') {
    return 'max-w-none'
  }

  if (presentation === 'sheet') {
    return 'max-w-none'
  }

  switch (size) {
    case 'sm':
      return 'max-w-md'
    case 'lg':
      return 'max-w-3xl'
    case 'md':
    default:
      return 'max-w-xl'
  }
}

function settleDialog(
  dialogs: ActiveWindowDialog[],
  dialogId: string,
  result?: unknown,
) {
  const target = dialogs.find((dialog) => dialog.id === dialogId)
  if (!target) {
    return dialogs
  }

  target.resolve(result)
  return dialogs.filter((dialog) => dialog.id !== dialogId)
}

function WindowDialogOverlay({
  dialog,
  surface,
  onClose,
}: {
  dialog: ActiveWindowDialog
  onClose: (result?: unknown) => void
  surface: WindowDialogSurface
}) {
  const presentation = resolvePresentation(
    surface,
    dialog.options.presentation ?? 'auto',
  )
  const size = dialog.options.size ?? 'md'
  const dismissible = dialog.options.dismissible ?? true
  const closeOnBackdrop = dialog.options.closeOnBackdrop ?? false
  const controls: WindowDialogControls = {
    close: (result) => onClose(result),
    dismiss: () => {
      if (dismissible) {
        onClose(undefined)
      }
    },
  }
  const isSheet = presentation === 'sheet'
  const isFullscreen = presentation === 'fullscreen' || size === 'fullscreen'

  return (
    <div className="absolute inset-0 z-[90]" data-testid="window-dialog-layer">
      <div
        aria-hidden="true"
        className="absolute inset-0 bg-[color:color-mix(in_srgb,var(--cp-shadow)_24%,transparent)] backdrop-blur-[2px]"
        data-testid="window-dialog-backdrop"
        onClick={() => {
          if (closeOnBackdrop && dismissible) {
            controls.dismiss()
          }
        }}
        onPointerDown={(event) => event.stopPropagation()}
      />
      <div
        className={clsx(
          'absolute inset-0 flex p-3 sm:p-4',
          isSheet ? 'items-end justify-stretch' : 'items-center justify-center',
        )}
        onPointerDown={(event) => event.stopPropagation()}
      >
        <section
          aria-modal="true"
          aria-label={dialog.options.title}
          role="dialog"
          data-testid="window-dialog"
          className={clsx(
            'relative flex max-h-full w-full flex-col overflow-hidden border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] shadow-[0_24px_64px_color-mix(in_srgb,var(--cp-shadow)_22%,transparent)]',
            isFullscreen
              ? 'h-full rounded-[16px] sm:rounded-[20px]'
              : isSheet
                ? 'rounded-[24px] rounded-b-[18px]'
                : 'rounded-[24px]',
            dialogSizeClass(size, presentation),
          )}
        >
          {dismissible ? (
            <button
              type="button"
              aria-label="Close dialog"
              className="absolute right-3 top-3 z-10 flex h-8 w-8 items-center justify-center rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_82%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_92%,transparent)] text-[color:var(--cp-muted)] transition-colors duration-150 hover:text-[color:var(--cp-text)]"
              onClick={() => controls.dismiss()}
            >
              <X className="size-4 stroke-[2]" />
            </button>
          ) : null}

          {dialog.options.title || dialog.options.description ? (
            <header className="border-b border-[color:color-mix(in_srgb,var(--cp-border)_74%,transparent)] px-5 pb-4 pt-5 pr-14">
              {dialog.options.title ? (
                <h2 className="font-display text-xl font-semibold text-[color:var(--cp-text)]">
                  {dialog.options.title}
                </h2>
              ) : null}
              {dialog.options.description ? (
                <p className="mt-2 text-sm leading-6 text-[color:var(--cp-muted)]">
                  {dialog.options.description}
                </p>
              ) : null}
            </header>
          ) : null}

          <div className="desktop-scrollbar min-h-0 flex-1 overflow-y-auto px-5 py-5">
            {dialog.options.renderBody(controls)}
          </div>

          {dialog.options.renderActions ? (
            <footer className="flex flex-wrap items-center justify-end gap-3 border-t border-[color:color-mix(in_srgb,var(--cp-border)_74%,transparent)] px-5 py-4">
              {dialog.options.renderActions(controls)}
            </footer>
          ) : null}
        </section>
      </div>
    </div>
  )
}

export function WindowDialogProvider({
  children,
  permissions,
  surface,
}: {
  children: ReactNode
  permissions: WindowDialogPermissions
  surface: WindowDialogSurface
}) {
  const [dialogs, setDialogs] = useState<ActiveWindowDialog[]>([])
  const dialogsRef = useRef<ActiveWindowDialog[]>([])
  const nextIdRef = useRef(1)

  useEffect(() => {
    dialogsRef.current = dialogs
  }, [dialogs])

  useEffect(() => {
    return () => {
      dialogsRef.current.forEach((dialog) => dialog.resolve(undefined))
      dialogsRef.current = []
    }
  }, [])

  const api: WindowDialogApi = {
    canPresent: (presentation) =>
      isPresentationAllowed(
        permissions,
        resolvePresentation(surface, presentation),
      ),
    close: (dialogId, result) => {
      setDialogs((prev) => settleDialog(prev, dialogId, result))
    },
    dismiss: (dialogId) => {
      setDialogs((prev) => settleDialog(prev, dialogId))
    },
    dismissTop: () => {
      const topDialog = dialogsRef.current[dialogsRef.current.length - 1]
      if (topDialog) {
        setDialogs((prev) => settleDialog(prev, topDialog.id))
      }
    },
    isOpen: dialogs.length > 0,
    open: (options) =>
      new Promise((resolve, reject) => {
        const resolvedPresentation = resolvePresentation(
          surface,
          options.presentation ?? 'auto',
        )

        if (!isPresentationAllowed(permissions, resolvedPresentation)) {
          reject(new WindowDialogPermissionError(resolvedPresentation))
          return
        }

        const dialog: ActiveWindowDialog = {
          id: `window-dialog-${nextIdRef.current++}`,
          options,
          resolve: (result) => resolve(result as never),
        }

        setDialogs((prev) => [...prev, dialog])
      }),
    permissions,
    stackDepth: dialogs.length,
  }

  const activeDialog = dialogs[dialogs.length - 1]

  return (
    <WindowDialogContext.Provider value={api}>
      <>
        {children}
        {activeDialog ? (
          <WindowDialogOverlay
            dialog={activeDialog}
            onClose={(result) => api.close(activeDialog.id, result)}
            surface={surface}
          />
        ) : null}
      </>
    </WindowDialogContext.Provider>
  )
}

export function useWindowDialog() {
  const value = useContext(WindowDialogContext)

  if (!value) {
    throw new Error('useWindowDialog must be used within WindowDialogProvider')
  }

  return value
}

export function resolveWindowDialogPermissions(
  app: Pick<AppDefinition, 'tier'>,
): WindowDialogPermissions {
  // Mock policy: fullscreen dialogs are runtime-gated and reserved for system apps.
  return {
    fullscreen: app.tier === 'system',
  }
}
