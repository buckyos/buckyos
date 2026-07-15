import type { LogicalDirectoryRecord } from '../../datamodel/types'
import { Check } from 'lucide-react'

function normalizeMountPath(path: string) {
  const normalized = path.trim().replace(/\./g, '/').replace(/^\/+|\/+$/g, '')
  return normalized ? `/${normalized}` : '/'
}

export function LogicalMountTreePicker({ directories, selected, onToggle }: { directories: LogicalDirectoryRecord[]; selected: string[]; onToggle: (path: string) => void }) {
  const selectedPaths = new Set(selected.map(normalizeMountPath))
  return (
    <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_260px]">
      <div className="shell-scrollbar max-h-80 space-y-1 overflow-auto rounded-md border border-[color:var(--cp-border)] p-2">
        {directories.map((directory) => {
          const checked = selectedPaths.has(normalizeMountPath(directory.path))
          return (
            <button className="flex w-full cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-left text-xs hover:bg-[color:var(--cp-surface-2)]" key={directory.directory_key} onClick={() => onToggle(directory.path)} style={{ paddingLeft: `${8 + directory.path.split('/').filter(Boolean).length * 12}px` }} type="button">
              <span className={`grid h-4 w-4 place-items-center rounded border text-[10px] ${checked ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent)] text-white' : 'border-[color:var(--cp-border)]'}`}>{checked ? <Check size={12} /> : ''}</span>
              <span className="font-mono">{directory.path}</span>
            </button>
          )
        })}
      </div>
      <div className="rounded-md border border-[color:var(--cp-border)] p-2 text-xs">
        <div className="mb-2 font-semibold text-[color:var(--cp-muted)]">Selected paths ({selected.length})</div>
        <div className="shell-scrollbar max-h-72 space-y-1 overflow-auto">{selected.map((path) => <button className="block w-full rounded border border-[color:var(--cp-border)] px-2 py-1 text-left font-mono hover:border-[color:var(--cp-accent)]" key={path} onClick={() => onToggle(path)} type="button">{path}</button>)}</div>
      </div>
    </div>
  )
}
