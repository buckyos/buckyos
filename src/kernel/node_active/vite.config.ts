import { defineConfig } from "vite";
import { resolve } from "path";
import dts from 'vite-plugin-dts'


export default defineConfig({
  optimizeDeps: {
    exclude: ['@mapbox/node-pre-gyp', 'mock-aws-s3', 'aws-sdk', 'nock']
  },
  build: {
    minify: 'terser',
    sourcemap: true,
    rollupOptions: {
      external: ['@mapbox/node-pre-gyp','mock-aws-s3', 'aws-sdk', 'nock'],
    },
    lib: {
      entry: resolve(__dirname,"index.html"),  // 配置入口文件路径
      name: "node_active",
      fileName: "node_active",
      formats: ["es", "umd"], // 打包生成的格式
    },
  },
  resolve: {
    alias: {
      "@": __dirname  
    }
  },
  plugins: [dts()]
});