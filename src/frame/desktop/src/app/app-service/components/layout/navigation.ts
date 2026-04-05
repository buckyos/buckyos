/* ── App Service navigation types ── */

export type AppServicePage = 'home' | 'detail' | 'install'

export interface AppServiceNav {
  page: AppServicePage
  serviceId?: string
  installStep?: number
}
