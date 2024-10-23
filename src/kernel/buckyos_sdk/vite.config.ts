import { defineConfig } from "vite";
import { resolve } from "path";

export default defineConfig({
  build: {
    minify: 'terser',
    sourcemap: true,
    lib: {
      entry: resolve(__dirname,"src/index.ts"),  // 配置入口文件路径
      name: "buckyos",
      fileName: "buckyos",
      formats: ["es", "umd"], // 打包生成的格式
    },
  }
});