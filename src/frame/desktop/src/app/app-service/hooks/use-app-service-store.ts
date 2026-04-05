/* ── App Service store context ── */

import { createContext, useContext } from 'react'
import type { AppServiceMockStore } from '../mock/store'

export const AppServiceStoreContext = createContext<AppServiceMockStore | null>(null)

export function useAppServiceStore(): AppServiceMockStore {
  const ctx = useContext(AppServiceStoreContext)
  if (!ctx) throw new Error('useAppServiceStore must be used within AppServiceStoreContext.Provider')
  return ctx
}
