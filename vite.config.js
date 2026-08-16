import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";
import vue from "@vitejs/plugin-vue";

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  clearScreen: false,
  server: {
    host: "0.0.0.0",
    port: 31847,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  preview: {
    host: "0.0.0.0",
    port: 31847,
    strictPort: true,
  },
});
