import { createRouter, createWebHashHistory } from "vue-router";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    {
      path: "/",
      name: "live",
      component: () => import("./pages/LivePage.vue"),
    },
    {
      path: "/settings",
      name: "settings",
      component: () => import("./features/settings/SettingsPage.vue"),
    },
    {
      path: "/diagnostics",
      name: "diagnostics",
      component: () => import("./pages/DiagnosticsPage.vue"),
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
