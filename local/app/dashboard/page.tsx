"use client";
import {
  SubscriptionDashboardSurface,
  type DashboardSurfaceAdapter,
} from "@subboost/ui/dashboard/subscription-dashboard-surface";
import { readJsonResponse } from "@subboost/ui/product/client-response";
import type { RefreshSubscriptionResponse, Subscription, SubscriptionLink } from "@subboost/ui/dashboard/dashboard-types";
import { useRuntimeConfig } from "@local/lib/runtime-config";

function resolveLocalDashboardDownloadUrl(subscription: Subscription): string {
  try {
    const url = new URL(subscription.subscriptionUrl, window.location.href);
    if (url.pathname.includes("/api/subscriptions/")) {
      return `${window.location.origin}${url.pathname}${url.search}`;
    }
  } catch {}
  return subscription.subscriptionUrl;
}

const localDashboardAdapter: DashboardSurfaceAdapter = {
  loginHref: "/login",
  newSubscriptionHref: "/?newSubscription=1",
  templatesHref: "/templates",
  settingsHref: "/dashboard/settings",
  settingsDescription: "查看工作空间、订阅源安全和运行状态",
  fetchSubscriptions: async () => {
    const response = await fetch("/api/subscriptions");
    const data = await readJsonResponse<{ subscriptions?: Subscription[]; error?: string }>(response, "获取订阅失败");
    return Array.isArray(data.subscriptions) ? data.subscriptions : [];
  },
  deleteSubscription: async (id) => {
    const response = await fetch(`/api/subscriptions/${encodeURIComponent(id)}`, { method: "DELETE" });
    await readJsonResponse<{ error?: string }>(response, "删除失败");
  },
  refreshSubscription: async (id) => {
    const response = await fetch(`/api/subscriptions/${encodeURIComponent(id)}/refresh`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
    });
    const data = await readJsonResponse<RefreshSubscriptionResponse>(response, "刷新失败");
    return data;
  },
  updateSubscriptionSettings: async (id, payload) => {
    const response = await fetch(`/api/subscriptions/${encodeURIComponent(id)}`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
    await readJsonResponse<{ error?: string }>(response, "保存失败");
  },
  resetSubscriptionLink: async (id, token) => {
    const response = await fetch(`/api/subscriptions/${encodeURIComponent(id)}/reset-link`, {
      method: "POST",
      headers: { "If-Match": `"${token}"` },
    });
    const data = await readJsonResponse<{ subscription: SubscriptionLink }>(response, "重置订阅链接失败");
    return data.subscription;
  },
  resolveDownloadUrl: resolveLocalDashboardDownloadUrl,
};

export default function DashboardPage() {
  const runtime = useRuntimeConfig();
  if (!runtime.config) return <p className="p-6" role={runtime.error ? "alert" : "status"}>{runtime.error || "读取运行配置…"}</p>;
  return <SubscriptionDashboardSurface adapter={{ ...localDashboardAdapter, autoUpdateIntervalPolicy: runtime.config.autoUpdateIntervalPolicy }} />;
}
