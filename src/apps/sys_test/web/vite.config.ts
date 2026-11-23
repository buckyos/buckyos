import { defineConfig, searchForWorkspaceRoot } from "vite";
import { resolve } from "path";

export default defineConfig({
  server: { fs: { allow: [searchForWorkspaceRoot(__dirname), resolve(__dirname, "../../shared")] } },
  build: {
    outDir: resolve(__dirname, "./dist"),
    emptyOutDir: true,
    sourcemap: false,
    rollupOptions: {
      preserveEntrySignatures: "strict",
      output: { entryFileNames: chunk => chunk.name === "locale" ? "assets/systest-locale.js" : "assets/[name]-[hash].js" },
      input: {
        locale: resolve(__dirname, "src/i18n.ts"),
        index: resolve(__dirname, "index.html"),
      },
    },
  },
  resolve: {
    alias: {
      "@": __dirname,
    },
  },
});
