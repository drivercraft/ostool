import { createRouter, createWebHistory } from "vue-router";

import BoardEditorView from "@/views/BoardEditorView.vue";
import BoardsView from "@/views/BoardsView.vue";
import DtbView from "@/views/DtbView.vue";
import OverviewView from "@/views/OverviewView.vue";
import ServerView from "@/views/ServerView.vue";
import SessionsView from "@/views/SessionsView.vue";
import TftpView from "@/views/TftpView.vue";

export const router = createRouter({
  history: createWebHistory("/admin/"),
  routes: [
    { path: "/", redirect: "/overview" },
    {
      path: "/overview",
      name: "overview",
      component: OverviewView,
      meta: { title: "总览" },
    },
    {
      path: "/boards",
      name: "boards",
      component: BoardsView,
      meta: { title: "开发板" },
    },
    {
      path: "/boards/new",
      name: "board-new",
      component: BoardEditorView,
      meta: { title: "新建开发板" },
    },
    {
      path: "/boards/:boardId",
      name: "board-edit",
      component: BoardEditorView,
      meta: { title: "编辑开发板" },
    },
    {
      path: "/dtbs",
      name: "dtbs",
      component: DtbView,
      meta: { title: "DTB 管理" },
    },
    {
      path: "/sessions",
      name: "sessions",
      component: SessionsView,
      meta: { title: "会话租约" },
    },
    {
      path: "/tftp",
      name: "tftp",
      component: TftpView,
      meta: { title: "TFTP 配置" },
    },
    {
      path: "/server",
      name: "server",
      component: ServerView,
      meta: { title: "Server 配置" },
    },
  ],
});
