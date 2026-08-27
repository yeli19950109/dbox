import { createRouter, createWebHashHistory } from "vue-router";
import ToolsView from "../views/ToolsView.vue";
import RunsView from "../views/RunsView.vue";
import ConfirmView from "../views/ConfirmView.vue";
import SettingsView from "../views/SettingsView.vue";
import SkillsView from "../views/SkillsView.vue";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", redirect: "/tools" },
    { path: "/tools", name: "tools", component: ToolsView },
    { path: "/runs", name: "runs", component: RunsView },
    { path: "/runs/confirm", name: "confirm", component: ConfirmView },
    { path: "/settings", name: "settings", component: SettingsView },
    { path: "/skills", name: "skills", component: SkillsView },
    { path: "/:pathMatch(.*)*", redirect: "/tools" },
  ],
});
