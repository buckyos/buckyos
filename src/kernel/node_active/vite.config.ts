import { defineConfig } from "vite";
import { resolve } from "path";
import react from "@vitejs/plugin-react";
import dts from "vite-plugin-dts";


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
      "@": resolve(__dirname, "src"),
      "@legacy": __dirname
    }
  },
  plugins: [react(), dts()]
});
