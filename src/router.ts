import { createRouter, createWebHashHistory } from "vue-router";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    {
      path: "/",
      name: "captioning",
      component: () => import("./features/captioning/CaptioningPage.vue"),
    },
    {
      path: "/settings",
      name: "settings",
      component: () => import("./features/settings/SettingsPage.vue"),
    },
    {
      path: "/diagnostics",
      name: "diagnostics",
      component: () => import("./features/diagnostics/DiagnosticsPage.vue"),
    },
    {
      path: "/:pathMatch(.*)*",
      name: "not-found",
      component: () => import("./pages/NotFoundPage.vue"),
    },
  ],
  scrollBehavior() {
    return { top: 0 };
  },
});
