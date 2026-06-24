import { createRouter, createWebHistory } from "vue-router";

import AdminLayout from "@/layouts/AdminLayout.vue";
import PublicLayout from "@/layouts/PublicLayout.vue";
import UserLayout from "@/layouts/UserLayout.vue";
import { useAuthStore } from "@/stores/auth";

const HomeView = () => import("@/views/public/HomeView.vue");
const ResourcesView = () => import("@/views/public/ResourcesView.vue");
const DocsView = () => import("@/views/public/DocsView.vue");
const LoginView = () => import("@/views/public/LoginView.vue");
const RegisterView = () => import("@/views/public/RegisterView.vue");
const TermsView = () => import("@/views/public/TermsView.vue");
const PrivacyView = () => import("@/views/public/PrivacyView.vue");
const DashboardView = () => import("@/views/user/DashboardView.vue");
const AccountView = () => import("@/views/user/AccountView.vue");
const MyLeasesView = () => import("@/views/user/MyLeasesView.vue");
const UserLeaseCreateView = () => import("@/views/user/UserLeaseCreateView.vue");
const OverviewView = () => import("@/views/admin/OverviewView.vue");
const BoardsView = () => import("@/views/admin/BoardsView.vue");
const BoardEditorView = () => import("@/views/admin/BoardEditorView.vue");
const DtbView = () => import("@/views/admin/DtbView.vue");
const SessionsView = () => import("@/views/admin/SessionsView.vue");
const LeasesView = () => import("@/views/admin/LeasesView.vue");
const LeaseEditorView = () => import("@/views/admin/LeaseEditorView.vue");
const UsersView = () => import("@/views/admin/UsersView.vue");
const RolesView = () => import("@/views/admin/RolesView.vue");
const TftpView = () => import("@/views/admin/TftpView.vue");
const ServerView = () => import("@/views/admin/ServerView.vue");

export const router = createRouter({
  history: createWebHistory("/"),
  scrollBehavior(to, _from, savedPosition) {
    if (savedPosition) {
      return savedPosition;
    }
    if (to.hash) {
      return {
        el: to.hash,
        top: 88,
        behavior: "smooth",
      };
    }
    return { top: 0 };
  },
  routes: [
    {
      path: "/",
      component: PublicLayout,
      children: [
        {
          path: "",
          name: "home",
          component: HomeView,
          meta: { title: "首页" },
        },
        {
          path: "resources",
          name: "resources",
          component: ResourcesView,
          meta: { title: "资源" },
        },
        {
          path: "docs",
          name: "docs",
          component: DocsView,
          meta: { title: "文档" },
        },
        {
          path: "login",
          name: "login",
          component: LoginView,
          meta: { title: "登录", publicOnly: true },
        },
        {
          path: "register",
          name: "register",
          component: RegisterView,
          meta: { title: "注册" },
        },
        {
          path: "terms",
          name: "terms",
          component: TermsView,
          meta: { title: "用户协议" },
        },
        {
          path: "privacy",
          name: "privacy",
          component: PrivacyView,
          meta: { title: "隐私政策" },
        },
        {
          path: "leases/new",
          name: "user-lease-new",
          component: UserLeaseCreateView,
          meta: { title: "申请租赁", requiresUser: true },
        },
      ],
    },
    {
      path: "/dashboard",
      component: UserLayout,
      meta: { requiresUser: true },
      children: [
        {
          path: "",
          name: "dashboard",
          component: DashboardView,
          meta: { title: "用户控制台", requiresUser: true },
        },
        {
          path: "account",
          name: "user-account",
          component: AccountView,
          meta: { title: "账户信息", requiresUser: true },
        },
        {
          path: "leases",
          name: "user-leases",
          component: MyLeasesView,
          meta: { title: "我的租赁", requiresUser: true },
        },
      ],
    },
    {
      path: "/admin",
      component: AdminLayout,
      meta: { requiresAdmin: true },
      children: [
        {
          path: "",
          redirect: "/admin/overview",
        },
        {
          path: "overview",
          name: "admin-overview",
          component: OverviewView,
          meta: { title: "总览" },
        },
        {
          path: "resources",
          redirect: "/admin/resources/boards",
        },
        {
          path: "resources/boards",
          name: "admin-resource-boards",
          component: BoardsView,
          meta: { title: "资源管理 / 开发板配置" },
        },
        {
          path: "resources/boards/new",
          name: "admin-resource-board-new",
          component: BoardEditorView,
          meta: { title: "资源管理 / 新建开发板" },
        },
        {
          path: "resources/boards/:boardId",
          name: "admin-resource-board-edit",
          component: BoardEditorView,
          meta: { title: "资源管理 / 编辑开发板" },
        },
        {
          path: "resources/dtbs",
          name: "admin-resource-dtbs",
          component: DtbView,
          meta: { title: "资源管理 / DTB 配置" },
        },
        {
          path: "resources/tftp",
          name: "admin-resource-tftp",
          component: TftpView,
          meta: { title: "资源管理 / TFTP 配置" },
        },
        {
          path: "rentals",
          redirect: "/admin/rentals/leases",
        },
        {
          path: "rentals/leases",
          name: "admin-rental-leases",
          component: LeasesView,
          meta: { title: "租赁管理 / 租赁情况" },
        },
        {
          path: "rentals/leases/new",
          name: "admin-rental-lease-new",
          component: LeaseEditorView,
          meta: { title: "租赁管理 / 新增租赁" },
        },
        {
          path: "rentals/leases/:leaseId",
          name: "admin-rental-lease-edit",
          component: LeaseEditorView,
          meta: { title: "租赁管理 / 编辑租赁" },
        },
        {
          path: "rentals/sessions",
          name: "admin-rental-sessions",
          component: SessionsView,
          meta: { title: "租赁管理 / 会话租约" },
        },
        {
          path: "users",
          redirect: "/admin/users/list",
        },
        {
          path: "users/list",
          name: "admin-user-list",
          component: UsersView,
          meta: { title: "用户管理 / 用户列表" },
        },
        {
          path: "users/roles",
          name: "admin-user-roles",
          component: RolesView,
          meta: { title: "用户管理 / 角色与权限" },
        },
        {
          path: "users/roles/new",
          name: "admin-user-role-new",
          component: RolesView,
          meta: { title: "用户管理 / 新建角色" },
        },
        {
          path: "users/roles/:roleId",
          name: "admin-user-role-edit",
          component: RolesView,
          meta: { title: "用户管理 / 编辑角色" },
        },
        {
          path: "users/permissions",
          redirect: "/admin/users/roles",
        },
        {
          path: "settings",
          redirect: "/admin/settings/server",
        },
        {
          path: "settings/server",
          name: "admin-settings-server",
          component: ServerView,
          meta: { title: "系统设置 / 服务配置" },
        },
        {
          path: "boards",
          redirect: "/admin/resources/boards",
        },
        {
          path: "boards/new",
          redirect: "/admin/resources/boards/new",
        },
        {
          path: "boards/:boardId",
          redirect: (to) => `/admin/resources/boards/${to.params.boardId}`,
        },
        {
          path: "dtbs",
          redirect: "/admin/resources/dtbs",
        },
        {
          path: "sessions",
          redirect: "/admin/rentals/sessions",
        },
        {
          path: "leases",
          redirect: "/admin/rentals/leases",
        },
        {
          path: "leases/new",
          redirect: "/admin/rentals/leases/new",
        },
        {
          path: "tftp",
          redirect: "/admin/resources/tftp",
        },
        {
          path: "server",
          redirect: "/admin/settings/server",
        },
      ],
    },
    {
      path: "/:pathMatch(.*)*",
      redirect: "/",
    },
  ],
});

router.beforeEach(async (to) => {
  const auth = useAuthStore();
  if (!auth.loaded) {
    await auth.loadCurrentUser();
  }
  if (to.meta.requiresAdmin && !auth.isAdmin) {
    return { name: "login", query: { next: to.fullPath } };
  }
  if (to.meta.requiresUser && !auth.isAuthenticated) {
    return { name: "login", query: { next: to.fullPath } };
  }
  if (to.meta.publicOnly && auth.isAdmin) {
    return { name: "admin-overview" };
  }
  return true;
});
