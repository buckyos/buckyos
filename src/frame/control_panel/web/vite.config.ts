import { defineConfig } from 'vite'
import path from 'node:path'
import react from '@vitejs/plugin-react-swc'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, 'src'),
    },
  },
  server: {
    proxy: {
      '/kapi/control-panel': {
        target: 'http://127.0.0.1:3180',
        changeOrigin: true,
      },
      '/kapi/opendan': {
        target: 'http://127.0.0.1:3180',
        changeOrigin: true,
      },
      '/kapi/task-manager': {
        target: 'http://127.0.0.1:3180',
        changeOrigin: true,
      },
    },
  },
})
