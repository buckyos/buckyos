import { useOutletContext } from 'react-router-dom'
import type { ShellOutletContext } from '../layout/CloudConsoleShell'

export function useShellContext() {
  return useOutletContext<ShellOutletContext>()
}

export function formatDate(ms: number) {
  return new Intl.DateTimeFormat('en-US', {
    month: 'short',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(ms)
}

export function paginate<T>(rows: T[], page: number, pageSize: number) {
  const totalPages = Math.max(1, Math.ceil(rows.length / pageSize))
  const safePage = Math.min(page, totalPages)
  return {
    page: safePage,
    totalPages,
    rows: rows.slice((safePage - 1) * pageSize, safePage * pageSize),
  }
}
