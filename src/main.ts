import "./assets/css/main.css";

import { createApp } from "vue";
import ui from "@nuxt/ui/vue-plugin";
import App from "./App.vue";
import { router } from "./router";

createApp(App).use(router).use(ui).mount("#app");
