import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// GitHub Pages 子路径部署：https://congduan.github.io/NTex/
export default defineConfig({
  plugins: [vue()],
  base: '/NTex/',
})
