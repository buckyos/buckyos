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
      },
      // One script: nothing may be left to load after activation closes the port.
      output: { inlineDynamicImports: true },
    }

  },
  resolve: {
    alias: {
      "@": resolve(__dirname, "src"),
    }
  },
  plugins: [react(), dts()]
});
