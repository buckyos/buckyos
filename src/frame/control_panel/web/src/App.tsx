import { RouterProvider } from 'react-router-dom'
import AuthProvider from './auth/AuthProvider'
import { I18nProvider } from './i18n'
import router from './routes/router'

const App = () => {
  return (
    <I18nProvider>
      <AuthProvider>
        <RouterProvider router={router} />
      </AuthProvider>
    </I18nProvider>
  )
}

export default App
