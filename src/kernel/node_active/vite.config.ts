import { defineConfig } from "vite";
import { resolve } from "path";
import dts from 'vite-plugin-dts'


export default defineConfig({
  publicDir:'res',
  optimizeDeps: {
    exclude: ['@mapbox/node-pre-gyp', 'mock-aws-s3', 'aws-sdk', 'nock']
  },
  build: {
    minify: 'terser',
    sourcemap: true,
    rollupOptions: {
      external: ['@mapbox/node-pre-gyp','mock-aws-s3', 'aws-sdk', 'nock'],
    }

  },
  resolve: {
    alias: {
      "@": __dirname  
    }
  },
  plugins: [dts()]
});