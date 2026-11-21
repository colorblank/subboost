"use client";

import { Button } from "@subboost/ui/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@subboost/ui/components/ui/dialog";
import { SubscriptionSettingsFields } from "@subboost/ui/components/subscription/subscription-settings-fields";
import {
  getAutoUpdateIntervalPolicyMinLabel,
  resolveAutoUpdateIntervalPolicy,
  type AutoUpdateIntervalPolicy,
} from "@subboost/core/subscription/auto-update-interval";
import type { Subscription } from "./dashboard-types";
import {
  buildNodeQuotaWarning,
  buildQuotaDisabledRecoveryText,
  isNodeQuotaAutoUpdateDisabled,
} from "./dashboard-auto-update-warning";

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  subscription: Subscription | null;
  settingsName: string;
  setSettingsName: (value: string) => void;
  smartNodeMatchingEnabled: boolean;
  setSmartNodeMatchingEnabled: (value: boolean) => void;
  autoUpdateEnabled: boolean;
  setAutoUpdateEnabled: (value: boolean) => void;
  autoUpdateHours: number;
  setAutoUpdateHours: (value: number) => void;
  savingSettings: boolean;
  onSave: () => void;
  onResetLink?: () => void;
  resettingLink?: boolean;
  userIsAdmin: boolean;
  autoUpdatePolicy?: AutoUpdateIntervalPolicy;
};

export function SubscriptionSettingsDialog({
  open,
  onOpenChange,
  subscription,
  settingsName,
  setSettingsName,
  smartNodeMatchingEnabled,
  setSmartNodeMatchingEnabled,
  autoUpdateEnabled,
  setAutoUpdateEnabled,
  autoUpdateHours,
  setAutoUpdateHours,
  savingSettings,
  onSave,
  onResetLink,
  resettingLink = false,
  userIsAdmin,
  autoUpdatePolicy,
}: Props) {
  const policy = autoUpdatePolicy ?? resolveAutoUpdateIntervalPolicy(userIsAdmin);
  const busy = savingSettings || resettingLink;
  const quotaWarning = subscription ? buildNodeQuotaWarning(subscription.autoUpdateState) : null;
  const quotaDisabled = subscription ? isNodeQuotaAutoUpdateDisabled(subscription.autoUpdateState) : false;
  return (
    <Dialog open={open} onOpenChange={(next) => { if (!busy) onOpenChange(next); }}>
      <DialogContent className="flex flex-col overflow-hidden sm:max-w-md">
        <DialogHeader>
          <DialogTitle>订阅设置</DialogTitle>
          <DialogDescription>
            改名与自动更新配置（最小 {getAutoUpdateIntervalPolicyMinLabel(policy)}，按创建时间计时）
          </DialogDescription>
        </DialogHeader>

        <DialogBody className="space-y-4 py-2">
          <SubscriptionSettingsFields
            name={settingsName}
            onNameChange={setSettingsName}
            smartNodeMatchingEnabled={smartNodeMatchingEnabled}
            onSmartNodeMatchingChange={setSmartNodeMatchingEnabled}
            autoUpdateEnabled={autoUpdateEnabled}
            onAutoUpdateEnabledChange={setAutoUpdateEnabled}
            autoUpdateHours={autoUpdateHours}
            onAutoUpdateHoursChange={setAutoUpdateHours}
            autoUpdatePolicy={policy}
            disabled={busy}
          />

          {!autoUpdateEnabled && subscription?.autoUpdateState.disabledAt && subscription.autoUpdateState.disabledReason && (
            <div className="rounded-xl border border-amber-400/25 bg-amber-500/10 px-3.5 py-2.5 text-xs leading-relaxed text-amber-100">
              {quotaDisabled
                ? `自动更新已关闭：${buildQuotaDisabledRecoveryText(subscription.autoUpdateState)}`
                : `自动更新已关闭：${subscription.autoUpdateState.disabledReason}。当前可用配置仍会保留；检查订阅 URL 后可重新开启自动更新。`}
            </div>
          )}

          {autoUpdateEnabled && quotaWarning && (
            <div className="rounded-xl border border-amber-400/25 bg-amber-500/10 px-3.5 py-2.5 text-xs leading-relaxed text-amber-100">
              {quotaWarning}。当前可用配置仍会保留；恢复到额度内后，下一次成功更新会自动清除这条警告。
            </div>
          )}
        </DialogBody>

        <DialogFooter className="flex-row flex-wrap items-center justify-between gap-2 sm:justify-between">
          {onResetLink && (
            <Button variant="outline" className="border-red-400/60 text-red-400 hover:border-red-400 hover:bg-red-500/10 hover:text-red-300" onClick={onResetLink} disabled={busy}>
              {resettingLink ? "重置中..." : "重置订阅链接"}
            </Button>
          )}
          <div className="ml-auto flex gap-2">
            <Button variant="outline" onClick={() => onOpenChange(false)} disabled={busy}>取消</Button>
            <Button onClick={onSave} disabled={busy}>{savingSettings ? "保存中..." : "保存"}</Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
