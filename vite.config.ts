import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import ui from "@nuxt/ui/vite";

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [
    vue(),
    ui({
      dts: false,
      router: false,
    }),
  ],

  // Keep Rust/Tauri errors visible in the same terminal.
  clearScreen: false,
  server: {
    // Must match `build.devUrl` in `src-tauri/tauri.conf.json`.
    port: 5173,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
