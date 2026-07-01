import { createPinia } from "pinia";
import { createApp } from "vue";

import App from "./App.vue";
import { router } from "./router";
import { useUiStore } from "./stores/ui";
import "./styles/app.css";

const app = createApp(App);
const pinia = createPinia();

app.use(pinia);
app.use(router);

useUiStore().initializeTheme();

app.mount("#app");
