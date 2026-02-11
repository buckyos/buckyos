import type { FormEvent, ReactNode } from 'react'
import { useCallback, useEffect, useMemo, useState } from 'react'

import { fetchSysConfigTree } from '@/api'

type SystemConfigTreeViewerProps = {
  defaultKey?: string
  depth?: number
  compact?: boolean
}

const BranchIcon = () => (
  <svg viewBox="0 0 16 16" className="size-3.5 text-[var(--cp-primary-strong)]" aria-hidden>
    <path
      d="M2.5 3.5h4l2 2h5v7h-11z"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.25"
      strokeLinejoin="round"
    />
  </svg>
)

const LeafIcon = () => (
  <svg viewBox="0 0 16 16" className="size-3.5 text-[var(--cp-muted)]" aria-hidden>
    <circle cx="8" cy="8" r="2.25" fill="currentColor" />
  </svg>
)

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value)

const renderTreeNodes = (node: Record<string, unknown>, parentPath = '', level = 0): ReactNode => {
  const entries = Object.entries(node)
  if (!entries.length) {
    return <p className="text-xs text-[var(--cp-muted)]">No nested keys.</p>
  }

  return (
    <div className="space-y-1.5">
      {entries.map(([name, child]) => {
        const path = parentPath ? `${parentPath}/${name}` : name
        const childMap = isRecord(child) ? child : {}
        const hasChildren = Object.keys(childMap).length > 0

        return (
          <div key={path} className={level > 0 ? 'ml-5' : ''}>
            <div className="min-w-0 rounded-md px-2 py-1.5 transition-colors hover:bg-white/70">
              <div className="flex min-w-0 items-center gap-2" title={path}>
                <span className="mt-0.5 shrink-0">{hasChildren ? <BranchIcon /> : <LeafIcon />}</span>
                <div className="min-w-0">
                  <p className="truncate text-xs font-semibold text-[var(--cp-ink)]">{name}</p>
                </div>
              </div>
            </div>
            {hasChildren ? (
              <div className="mt-1 pl-4">
                {renderTreeNodes(childMap, path, level + 1)}
              </div>
            ) : null}
          </div>
        )
      })}
    </div>
  )
}

const SystemConfigTreeViewer = ({
  defaultKey = '',
  depth = 4,
  compact = false,
}: SystemConfigTreeViewerProps) => {
  const [inputKey, setInputKey] = useState(defaultKey)
  const [activeKey, setActiveKey] = useState(defaultKey)
  const [treeData, setTreeData] = useState<SysConfigTreeResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [errorMessage, setErrorMessage] = useState<string | null>(null)

  const loadTree = useCallback(async (key: string) => {
    const normalized = key.trim()
    setLoading(true)
    const { data, error } = await fetchSysConfigTree(normalized, depth)

    if (error) {
      const message =
        error instanceof Error
          ? error.message
          : typeof error === 'string'
            ? error
            : 'System config tree request failed.'
      setErrorMessage(message)
    } else {
      setErrorMessage(null)
    }

    setTreeData(data)
    setLoading(false)
  }, [depth])

  useEffect(() => {
    loadTree(activeKey)
  }, [activeKey, loadTree])

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setActiveKey(inputKey.trim())
  }

  const treeNode = useMemo(() => {
    if (!treeData?.tree || !isRecord(treeData.tree)) {
      return null
    }

    return renderTreeNodes(treeData.tree)
  }, [treeData])

  return (
    <div className="space-y-3">
      <form className="flex flex-wrap items-end gap-2" onSubmit={handleSubmit}>
        <label className="flex min-w-[200px] flex-1 flex-col gap-1 text-xs text-[var(--cp-muted)]">
          Config Key
          <input
            type="text"
            value={inputKey}
            onChange={(event) => setInputKey(event.target.value)}
            placeholder="(root)"
            className="h-10 rounded-xl border border-[var(--cp-border)] bg-white px-3 text-sm text-[var(--cp-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--cp-primary)]"
          />
        </label>
        <button
          type="submit"
          className="h-10 rounded-full bg-[var(--cp-primary)] px-4 text-xs font-semibold text-white transition hover:bg-[var(--cp-primary-strong)]"
        >
          Reload
        </button>
      </form>

      {errorMessage ? (
        <div className="rounded-xl border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
          {errorMessage}
        </div>
      ) : null}

      <div className={`overflow-auto rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] p-3 ${compact ? 'max-h-72' : 'max-h-[32rem]'}`}>
        {loading ? (
          <p className="text-xs text-[var(--cp-muted)]">Loading system config tree...</p>
        ) : treeNode ? (
          <div className="min-w-0">{treeNode}</div>
        ) : (
          <p className="text-xs text-[var(--cp-muted)]">No tree data available.</p>
        )}
      </div>

      <p className="text-[11px] text-[var(--cp-muted)]">
        Key: {(treeData?.key ?? activeKey) || '(root)'} · Depth: {treeData?.depth ?? depth} · Showing top levels
      </p>
    </div>
  )
}

export default SystemConfigTreeViewer
