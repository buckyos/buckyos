import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react-swc'

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:4070',
        changeOrigin: true,
      },
      '/kapi': {
        target: 'http://127.0.0.1:4070',
        changeOrigin: true,
      },
    },
  },
})
