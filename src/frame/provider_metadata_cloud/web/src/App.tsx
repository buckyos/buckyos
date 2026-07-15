import { RouterProvider } from 'react-router-dom'
import { I18nProvider } from './i18n/provider'
import { ProviderMetadataStoreProvider } from './state/ProviderMetadataStore'
import { ThemeProvider } from './theme/provider'
import { router } from './routes'

export default function App() {
  return (
    <I18nProvider>
      <ThemeProvider>
        <ProviderMetadataStoreProvider>
          <RouterProvider router={router} />
        </ProviderMetadataStoreProvider>
      </ThemeProvider>
    </I18nProvider>
  )
}
