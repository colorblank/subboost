"use client";

import { HomeSurface, type HomeSurfaceAdapter } from "@subboost/ui/product/home/home-surface";
import { readJsonResponse, readSourceImportResponse } from "@subboost/ui/product/client-response";
import { createRulesProductApi, type QuickTemplateDetail } from "@subboost/ui/product/api-adapter";
import { useRuntimeConfig } from "@local/lib/runtime-config";

type TemplateListResponse = {
  templates?: Array<{ id: string; name?: string; description?: string }>;
};

const localHomeAdapter: HomeSurfaceAdapter = {
  loginHref: "/login",
  templateUploadHref: "/templates?upload=1",
  renderNotice: ({ user }) => (
    <aside className="mb-4 flex flex-col gap-3 rounded-xl border border-ink/10 bg-ink/5 px-4 py-3 text-sm sm:flex-row sm:items-start sm:justify-between" aria-label="本地工作空间">
      <div className="min-w-0">
        <p className="font-medium text-fg">{user?.isGuest ? "游客工作空间 · 无需注册" : "本地工作空间"}</p>
        <p className="mt-1 text-xs leading-relaxed text-fg-60">
          订阅和模板保存在本地 SQLite，游客数据按浏览器 Cookie 隔离。请保留 Cookie；订阅链接持有者可读取配置，请勿公开分享。
        </p>
      </div>
      <details className="shrink-0 text-xs text-fg-60 sm:max-w-xs">
        <summary className="cursor-pointer rounded py-1 font-medium text-fg-70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-4">
          终端与 AI 工具 · CLI / MCP
        </summary>
        <div className="mt-2 space-y-2">
          <p>命令行转换：<code className="break-all">subboost convert --help</code></p>
          <p>原生 MCP stdio：<code className="break-all">subboost mcp</code></p>
          <p>CLI / MCP 使用独立的本地空间，不会读取浏览器游客数据。</p>
        </div>
      </details>
    </aside>
  ),
  productApi: {
    sourceImport: {
      importSource: async (request) => {
        const data = await readSourceImportResponse(
          await fetch("/api/source-import", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(request),
          })
        );
        return {
          content: typeof data.content === "string" ? data.content : "",
          headers: data.headers || {},
          parseResult: data.parseResult,
        };
      },
    },
    templates: {
      catalogEnabled: false,
      builtinEngagementEnabled: false,
      loadMyTemplates: async () => {
        const data = await readJsonResponse<TemplateListResponse>(
          await fetch("/api/templates?type=my", { cache: "no-store" })
        );
        return (data.templates || []).map((template) => ({
          id: template.id,
          name: template.name || "未命名模板",
          description: template.description || "",
        }));
      },
      loadTemplateDetail: async (id) => {
        const response = await fetch(`/api/templates/${encodeURIComponent(id)}`, { cache: "no-store" });
        if (response.status === 404) return null;
        const data = await readJsonResponse<{ template?: QuickTemplateDetail }>(response);
        return data.template ?? null;
      },
    },
    rules: createRulesProductApi(),
  },
  loadSubscription: (id) => fetch(`/api/subscriptions/${encodeURIComponent(id)}`, { cache: "no-store" }),
  subscription: {
    loginHref: "/login",
    saveSubscription: ({ isEditing, subscriptionId, payload }) => {
      const endpoint =
        isEditing && subscriptionId
          ? `/api/subscriptions/${encodeURIComponent(subscriptionId)}`
          : "/api/subscriptions";
      return fetch(endpoint, {
        method: isEditing ? "PUT" : "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
    },
  },
};

export default function Page() {
  const runtime = useRuntimeConfig();
  if (!runtime.config) return <p className="p-6" role={runtime.error ? "alert" : "status"}>{runtime.error || "读取运行配置…"}</p>;
  const adapter: HomeSurfaceAdapter = {
    ...localHomeAdapter,
    subscription: { ...localHomeAdapter.subscription!, autoUpdateIntervalPolicy: runtime.config.autoUpdateIntervalPolicy },
  };
  return <HomeSurface adapter={adapter} />;
}
