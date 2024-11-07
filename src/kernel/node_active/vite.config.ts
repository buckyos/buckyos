import { defineConfig } from "vite";
import { resolve } from "path";
import dts from 'vite-plugin-dts'


export default defineConfig({
  root: '.',
  publicDir:'res',
  build: {
    outDir: 'dist',
    minify: false,
    sourcemap: true,
    rollupOptions: {
      input: {
        main: "index.html",
      }
    }

  },
  resolve: {
    alias: {
      "@": __dirname  
    }
  },
  plugins: [dts()]
});