import { createContext, useContext } from 'react'

export interface ToastSpec {
  text: string
  tone?: 'info' | 'success' | 'warning'
  action?: { label: string; onClick: () => void }
}

export const ToastContext = createContext<(toast: ToastSpec) => void>(() => {})

export function useToast() {
  return useContext(ToastContext)
}
