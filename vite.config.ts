import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { NuxtIconBundle } from "@nuxt/icon/vite";
import ui from "@nuxt/ui/vite";

const NUXT_UI_RUNTIME_ICONS = [
  "lucide:check",
  "lucide:chevron-down",
  "lucide:chevron-up",
  "lucide:loader-circle",
  "lucide:minus",
  "lucide:plus",
  "lucide:x",
];

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [
    vue(),
    NuxtIconBundle({
      icons: NUXT_UI_RUNTIME_ICONS,
      scan: {
        globInclude: ["src/**/*.{vue,ts}"],
      },
      sizeLimitKb: 32,
    }),
    ui(),
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
