import { Database, FileCode, Link2, Server } from "lucide-react";
import { Card, CardContent } from "@subboost/ui/components/ui/card";

type DashboardStatsUser = {
  isAdmin?: boolean;
  templateCount: number;
  quota: {
    maxSubscriptions: number;
    maxNodesPerSubscription: number;
    maxCustomTemplates: number;
    maxImportSourcesPerType: number;
  };
};

export function DashboardStatsCards({
  subscriptionCount,
  user,
}: {
  subscriptionCount: number;
  user: DashboardStatsUser;
}) {
  return (
    <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 mb-6">
      <Card className="p-5">
        <CardContent className="p-0">
          <div className="flex items-center gap-3.5">
            <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-primary-500/20 text-primary-500">
              <FileCode className="h-5 w-5" />
            </div>
            <div className="min-w-0">
              <p className="text-xs font-medium text-fg-50">订阅配额</p>
              <p className="mt-0.5 text-xl font-bold tabular-nums sm:text-2xl">
                {subscriptionCount}
                <span className="ml-0.5 text-xs font-normal text-fg-50">/{user.quota.maxSubscriptions}</span>
              </p>
            </div>
          </div>
        </CardContent>
      </Card>

      <Card className="p-5">
        <CardContent className="p-0">
          <div className="flex items-center gap-3.5">
            <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-purple-500/20 text-purple-500">
              <Server className="h-5 w-5" />
            </div>
            <div className="min-w-0">
              <p className="text-xs font-medium text-fg-50">节点上限配额</p>
              <p className="mt-0.5 text-xl font-bold tabular-nums sm:text-2xl">
                {user.quota.maxNodesPerSubscription}
                <span className="ml-0.5 text-xs font-normal text-fg-50">/订阅</span>
              </p>
            </div>
          </div>
        </CardContent>
      </Card>

      <Card className="p-5">
        <CardContent className="p-0">
          <div className="flex items-center gap-3.5">
            <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-green-500/20 text-green-500">
              <Database className="h-5 w-5" />
            </div>
            <div className="min-w-0">
              <p className="text-xs font-medium text-fg-50">模板配额</p>
              <p className="mt-0.5 text-xl font-bold tabular-nums sm:text-2xl">
                {user.templateCount}
                <span className="ml-0.5 text-xs font-normal text-fg-50">/{user.quota.maxCustomTemplates}</span>
              </p>
            </div>
          </div>
        </CardContent>
      </Card>

      <Card className="p-5">
        <CardContent className="p-0">
          <div className="flex items-center gap-3.5">
            <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-blue-500/20 text-blue-500">
              <Link2 className="h-5 w-5" />
            </div>
            <div className="min-w-0">
              <p className="text-xs font-medium text-fg-50">导入源配额</p>
              <p className="mt-0.5 text-xl font-bold tabular-nums sm:text-2xl">
                {user.isAdmin || user.quota.maxImportSourcesPerType >= 9999 ? "不限" : user.quota.maxImportSourcesPerType}
                <span className="ml-0.5 text-xs font-normal text-fg-50">/每种</span>
              </p>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
