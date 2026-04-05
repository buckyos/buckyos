import {
  useMemo,
  useState,
} from 'react'
import { useMediaQuery } from '@mui/material'
import {
  Filemanager,
  Willow,
  WillowDark,
  type IEntity,
} from '@svar-ui/react-filemanager'
import '@svar-ui/react-filemanager/all.css'
import { useI18n } from '../../i18n/provider'
import type { LayoutState, ThemeMode } from '../../models/ui'
import {
  createExtraInfo,
  createFilePreview,
  createFilesEntities,
} from './data'

function joinPath(parent: string, name: string) {
  return parent === '/' ? `/${name}` : `${parent}/${name}`
}

function getParentPath(id: string) {
  const parts = id.split('/').filter(Boolean)

  if (parts.length <= 1) {
    return '/'
  }

  return `/${parts.slice(0, -1).join('/')}`
}

function getBaseName(id: string) {
  return id.split('/').filter(Boolean).at(-1) ?? id
}

function isSameOrChildPath(candidate: string, root: string) {
  return candidate === root || candidate.startsWith(`${root}/`)
}

function dedupeRootIds(ids: string[]) {
  return ids.filter((id) => !ids.some((other) => other !== id && isSameOrChildPath(id, other)))
}

function replacePathPrefix(id: string, from: string, to: string) {
  if (id === from) {
    return to
  }

  return `${to}${id.slice(from.length)}`
}

function resolveInitialPath(initialPath: string | undefined, files: IEntity[]) {
  if (!initialPath) {
    return '/Desktop'
  }

  return files.some((item) => item.id === initialPath) ? initialPath : '/Desktop'
}

export function FilesView({
  embedded,
  initialPath,
  layoutState,
  locale,
  runtimeContainer,
  themeMode,
}: {
  embedded: boolean
  initialPath?: string
  layoutState?: LayoutState
  locale: string
  runtimeContainer?: string
  themeMode: ThemeMode
}) {
  const { t } = useI18n()
  const isCompact = useMediaQuery('(max-width: 960px)')
  const [files, setFiles] = useState<IEntity[]>(() => createFilesEntities({
    layoutState,
    locale,
    runtimeContainer,
  }))
  const [lastAction, setLastAction] = useState<string | null>(null)

  const ThemeWrapper = themeMode === 'dark' ? WillowDark : Willow
  const usedBytes = useMemo(
    () => files.reduce((total, item) => total + (item.size ?? 0), 0),
    [files],
  )
  const resolvedPath = useMemo(
    () => resolveInitialPath(initialPath, files),
    [files, initialPath],
  )
  const panels = useMemo(
    () => [{ path: resolvedPath, selected: [] }],
    [resolvedPath],
  )
  const viewMode = isCompact ? 'cards' : 'table'

  return (
    <section
      data-embedded={embedded ? 'true' : 'false'}
      className="files-browser relative flex h-full min-h-0 flex-col overflow-hidden"
    >
      <ThemeWrapper fonts={false}>
        <div className="h-full">
          <Filemanager
            key={`${resolvedPath}:${viewMode}`}
            data={files}
            drive={{ total: 512_000_000_000, used: usedBytes }}
            extraInfo={createExtraInfo}
            mode={viewMode}
            panels={panels}
            preview={!isCompact}
            previews={createFilePreview}
            onCreateFile={({ file, parent, newId }) => {
              const nextId = newId ?? joinPath(parent, file.name)

              setFiles((prev) => [
                ...prev,
                {
                  id: nextId,
                  type: file.type ?? 'file',
                  date: file.date ?? new Date(),
                  size: file.size ?? 0,
                  owner: 'user',
                  scope: 'workspace',
                  note: 'Created from the Files app.',
                },
              ])
              setLastAction(t('files.actionCreated', undefined, { name: file.name }))
            }}
            onDeleteFiles={({ ids }) => {
              const roots = dedupeRootIds(ids)

              setFiles((prev) =>
                prev.filter((item) => !roots.some((id) => isSameOrChildPath(item.id, id))),
              )
              setLastAction(t('files.actionDeleted', undefined, { count: roots.length }))
            }}
            onRenameFile={({ id, name, newId }) => {
              const targetId = newId ?? joinPath(getParentPath(id), name)

              setFiles((prev) => prev.map((item) => {
                if (!isSameOrChildPath(item.id, id)) {
                  return item
                }

                return {
                  ...item,
                  id: replacePathPrefix(item.id, id, targetId),
                }
              }))
              setLastAction(t('files.actionRenamed', undefined, { name }))
            }}
            onMoveFiles={({ ids, target, newIds }) => {
              const roots = dedupeRootIds(ids)

              setFiles((prev) => prev.map((item) => {
                const movingIndex = roots.findIndex((id) => isSameOrChildPath(item.id, id))

                if (movingIndex < 0) {
                  return item
                }

                const nextRoot = newIds?.[movingIndex] ?? joinPath(target, getBaseName(roots[movingIndex]))
                return {
                  ...item,
                  id: replacePathPrefix(item.id, roots[movingIndex], nextRoot),
                }
              }))
              setLastAction(t('files.actionMoved', undefined, { count: roots.length }))
            }}
            onCopyFiles={({ ids, target, newIds }) => {
              const roots = dedupeRootIds(ids)

              setFiles((prev) => {
                const copies = roots.flatMap((rootId, index) => {
                  const rootCopyId = newIds?.[index] ?? joinPath(target, getBaseName(rootId))

                  return prev
                    .filter((item) => isSameOrChildPath(item.id, rootId))
                    .map((item) => ({
                      ...item,
                      id: replacePathPrefix(item.id, rootId, rootCopyId),
                      date: new Date(),
                    }))
                })

                return [...prev, ...copies]
              })
              setLastAction(t('files.actionCopied', undefined, { count: roots.length }))
            }}
            onOpenFile={({ id }) => {
              setLastAction(t('files.actionOpened', undefined, { name: getBaseName(id) }))
            }}
            onDownloadFile={({ id }) => {
              setLastAction(t('files.actionDownloaded', undefined, { name: getBaseName(id) }))
            }}
          />
        </div>
      </ThemeWrapper>
      {lastAction ? (
        <div className="pointer-events-none absolute bottom-4 right-4 z-20">
          <span className="shell-pill px-3 py-1.5 text-xs text-[color:var(--cp-text)]">
            {lastAction}
          </span>
        </div>
      ) : null}
    </section>
  )
}
