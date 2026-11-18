"use client";

import * as React from "react";
import Link from "next/link";
import { LogOut, Network, ServerCog, ShieldCheck } from "lucide-react";

import { Button } from "@subboost/ui/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@subboost/ui/components/ui/card";
import { SwitchField } from "@subboost/ui/components/ui/switch-field";
import { useUserStore } from "@subboost/ui/store/user-store";

export default function SettingsPage() {
  const { user, fetchUser, logout } = useUserStore();
  const [allowUnsafeSubscriptionSources, setAllowUnsafeSubscriptionSources] = React.useState(false);
  const [sourceImportLoading, setSourceImportLoading] = React.useState(true);
  const [sourceImportSaving, setSourceImportSaving] = React.useState(false);
  const [sourceImportError, setSourceImportError] = React.useState<string | null>(null);

  React.useEffect(() => {
    void fetchUser();
  }, [fetchUser]);

  React.useEffect(() => {
    let cancelled = false;
    if (!user) {
      setSourceImportLoading(false);
      return () => {
        cancelled = true;
      };
    }

    setSourceImportLoading(true);
    setSourceImportError(null);
    void fetch("/api/settings/source-import", { cache: "no-store" })
      .then(async (response) => {
        if (!response.ok) throw new Error("Unable to load source import settings.");
        const body = (await response.json()) as { allowUnsafeSubscriptionSources?: unknown };
        if (typeof body.allowUnsafeSubscriptionSources !== "boolean") {
          throw new Error("Invalid source import settings response.");
        }
        if (!cancelled) setAllowUnsafeSubscriptionSources(body.allowUnsafeSubscriptionSources);
      })
      .catch(() => {
        if (!cancelled) setSourceImportError("加载失败，请刷新重试");
      })
      .finally(() => {
        if (!cancelled) setSourceImportLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [user]);

  const handleLogout = async () => {
    await logout();
    // Clear the mounted authenticated page and its cached state after logout.
    // eslint-disable-next-line @next/next/no-location-assign-relative-destination
    window.location.href = "/";
  };

  const handleUnsafeSourceToggle = async (checked: boolean) => {
    const previousValue = allowUnsafeSubscriptionSources;
    setAllowUnsafeSubscriptionSources(checked);
    setSourceImportSaving(true);
    setSourceImportError(null);

    try {
      const response = await fetch("/api/settings/source-import", {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ allowUnsafeSubscriptionSources: checked }),
      });
      if (!response.ok) throw new Error("Unable to save source import settings.");
      const body = (await response.json()) as { allowUnsafeSubscriptionSources?: unknown };
      if (typeof body.allowUnsafeSubscriptionSources !== "boolean") {
        throw new Error("Invalid source import settings response.");
      }
      setAllowUnsafeSubscriptionSources(body.allowUnsafeSubscriptionSources);
    } catch {
      setAllowUnsafeSubscriptionSources(previousValue);
      setSourceImportError("保存失败，请重试");
    } finally {
      setSourceImportSaving(false);
    }
  };

  return (
    <div className="page-container py-6 sm:py-8">
      <div className="mb-6 flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h1 className="text-xl font-bold tracking-tight text-fg sm:text-2xl">账户设置</h1>
          <p className="mt-1 text-sm text-fg-50">浏览器独立空间、订阅源安全和运行端点</p>
        </div>
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        <Card className="p-5 sm:p-6">
          <CardHeader className="flex flex-row items-center gap-3 space-y-0 pb-4">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-indigo-500/20 text-indigo-300">
              <ShieldCheck className="h-5 w-5" />
            </div>
            <CardTitle className="text-base sm:text-lg">{user?.isGuest ? "游客工作空间" : "本地管理员"}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div>
              <p className="text-xs text-fg-45">用户名</p>
              <p className="mt-1 text-sm font-medium text-fg">{user?.username || "正在连接工作空间…"}</p>
            </div>
            <div>
              <p className="text-xs text-fg-45">已保存订阅</p>
              <p className="mt-1 text-sm font-medium tabular-nums text-fg">{user ? `${user.subscriptionCount} / ${user.quota.maxSubscriptions}` : "-"}</p>
            </div>
            {user?.isGuest ? (
              <div className="space-y-3">
                <p className="text-sm leading-relaxed text-fg-60">无需登录即可保存订阅和模板。此空间由当前浏览器 Cookie 识别，清除 Cookie 后将无法访问原有数据。</p>
                <Button asChild variant="outline"><Link href="/login">管理员登录（可选）</Link></Button>
              </div>
            ) : (
              <Button variant="destructive" onClick={() => void handleLogout()} disabled={!user}>
                <LogOut className="h-4 w-4" />
                退出登录
              </Button>
            )}
          </CardContent>
        </Card>

        <Card className="p-5 sm:p-6">
          <CardHeader className="flex flex-row items-center gap-3 space-y-0 pb-4">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-emerald-500/20 text-emerald-300">
              <Network className="h-5 w-5" />
            </div>
            <CardTitle className="text-base sm:text-lg">订阅源安全</CardTitle>
          </CardHeader>
          <CardContent className="space-y-2">
            <SwitchField
              label="允许本机和局域网订阅"
              description="开启后，本机、局域网及其他保留地址都可作为订阅源。仅在信任来源时开启。"
              checked={allowUnsafeSubscriptionSources}
              disabled={!user || sourceImportLoading || sourceImportSaving}
              onCheckedChange={(checked) => void handleUnsafeSourceToggle(checked)}
            />
            {sourceImportError && <p className="text-xs text-red-300">{sourceImportError}</p>}
          </CardContent>
        </Card>

        <Card className="p-5 sm:p-6">
          <CardHeader className="flex flex-row items-center gap-3 space-y-0 pb-4">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-sky-500/20 text-sky-300">
              <ServerCog className="h-5 w-5" />
            </div>
            <CardTitle className="text-base sm:text-lg">运行端点</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm text-fg-60">
            <div>
              <p className="text-xs text-fg-45">存活检查</p>
              <code className="mt-1 block rounded-lg border border-ink/10 bg-ink/5 px-3 py-2 font-mono text-xs text-fg-70">/api/health/live</code>
            </div>
            <div>
              <p className="text-xs text-fg-45">就绪检查</p>
              <code className="mt-1 block rounded-lg border border-ink/10 bg-ink/5 px-3 py-2 font-mono text-xs text-fg-70">/api/health/ready</code>
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
