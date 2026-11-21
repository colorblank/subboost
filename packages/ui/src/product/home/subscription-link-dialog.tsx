"use client";

import type * as React from "react";
import { Check, Copy, Link as LinkIcon, Loader2 } from "lucide-react";
import { Button } from "@subboost/ui/components/ui/button";
import { IconButton } from "@subboost/ui/components/ui/icon-button";
import { Input } from "@subboost/ui/components/ui/input";
import { SubscriptionSettingsFields } from "@subboost/ui/components/subscription/subscription-settings-fields";
import { SubscriptionQrCode } from "@subboost/ui/product/subscription/subscription-qr-code";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@subboost/ui/components/ui/dialog";
import {
  getAutoUpdateIntervalPolicyMinLabel,
  type AutoUpdateIntervalPolicy,
} from "@subboost/core/subscription/auto-update-interval";

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  subscriptionUrl: string;
  subscriptionName: string;
  setSubscriptionName: (value: string) => void;
  autoUpdateEnabled: boolean;
  setAutoUpdateEnabled: (value: boolean) => void;
  autoUpdateHours: number;
  setAutoUpdateHours: (value: number) => void;
  autoUpdatePolicy: AutoUpdateIntervalPolicy;
  smartNodeMatchingEnabled: boolean;
  setSmartNodeMatchingEnabled: (value: boolean) => void;
  isCreatingSubscription: boolean;
  copied: boolean;
  isEditingExistingSubscription: boolean;
  handleCopyUrl: () => void;
  handleCreateSubscription: () => void;
};

export function SubscriptionLinkDialog({
  open,
  onOpenChange,
  subscriptionUrl,
  subscriptionName,
  setSubscriptionName,
  autoUpdateEnabled,
  setAutoUpdateEnabled,
  autoUpdateHours,
  setAutoUpdateHours,
  autoUpdatePolicy,
  smartNodeMatchingEnabled,
  setSmartNodeMatchingEnabled,
  isCreatingSubscription,
  copied,
  isEditingExistingSubscription,
  handleCopyUrl,
  handleCreateSubscription,
}: Props) {
  const close = () => onOpenChange(false);
  const minAutoUpdateLabel = getAutoUpdateIntervalPolicyMinLabel(autoUpdatePolicy);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex flex-col overflow-hidden sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <LinkIcon className="h-5 w-5 text-indigo-400" />
            {subscriptionUrl
              ? (isEditingExistingSubscription ? "订阅链接已更新" : "订阅链接已生成")
              : (isEditingExistingSubscription ? "更新订阅链接" : "生成订阅链接")}
          </DialogTitle>
          <DialogDescription>
            {subscriptionUrl
              ? "复制下方链接到 Clash 客户端导入使用"
              : isEditingExistingSubscription
                ? "将覆盖该订阅的配置与订阅源，链接保持不变"
                : "生成持久化的订阅链接，支持在 Clash 客户端中自动更新"}
          </DialogDescription>
        </DialogHeader>

        {!subscriptionUrl ? (
          <DialogBody className="space-y-4 py-4">
            <SubscriptionSettingsFields
              name={subscriptionName}
              onNameChange={setSubscriptionName}
              smartNodeMatchingEnabled={smartNodeMatchingEnabled}
              onSmartNodeMatchingChange={setSmartNodeMatchingEnabled}
              autoUpdateEnabled={autoUpdateEnabled}
              onAutoUpdateEnabledChange={setAutoUpdateEnabled}
              autoUpdateHours={autoUpdateHours}
              onAutoUpdateHoursChange={setAutoUpdateHours}
              autoUpdatePolicy={autoUpdatePolicy}
              autoUpdateDescription="开启后服务器会按设定间隔刷新缓存"
              autoUpdateHoursDescription={`最小 ${minAutoUpdateLabel}，按创建时间计时`}
            />

            <div className="p-3 rounded-lg bg-amber-500/10 border border-amber-500/30 text-sm text-amber-200">
              <p className="font-medium mb-1">注意事项</p>
              <ul className="text-xs text-warning-muted-fg space-y-1">
                <li>🔒 配置数据将加密存储于服务器</li>
                <li>🔑 订阅链接相当于访问凭证，请勿公开分享</li>
                <li>⏱️ 客户端高频拉取订阅会被封禁，请合理配置</li>
                {isEditingExistingSubscription ? (
                  <>
                    <li>⚠️ 更新将覆盖原订阅配置与订阅源</li>
                    <li>✅ 订阅链接保持不变（无需在客户端重新导入）</li>
                  </>
                ) : (
                  <li>🗑️ 您可以随时在仪表盘删除订阅</li>
                )}
              </ul>
            </div>
          </DialogBody>
        ) : (
          <DialogBody className="space-y-4 py-4">
            <SubscriptionQrCode url={subscriptionUrl} />
            <div className="space-y-2">
              <p className="text-sm font-medium">订阅链接</p>
              <div className="flex gap-2">
                <Input value={subscriptionUrl} readOnly className="font-mono text-xs" />
                <IconButton
                  label={copied ? "已复制订阅链接" : "复制订阅链接"}
                  variant="outline"
                  onClick={handleCopyUrl}
                  className="flex-shrink-0"
                >
                  {copied ? (
                    <Check className="h-4 w-4 text-green-400" />
                  ) : (
                    <Copy className="h-4 w-4" />
                  )}
                </IconButton>
              </div>
            </div>

            <div className="p-3 rounded-lg bg-green-500/10 border border-green-500/30 text-sm">
              <p className="text-green-200 font-medium mb-1">
                ✅ {isEditingExistingSubscription ? "更新成功" : "创建成功"}
              </p>
              <p className="text-xs text-success-muted-fg">
                {isEditingExistingSubscription ? "订阅链接保持不变，可在仪表盘查看" : "您可以在仪表盘中管理所有订阅"}
              </p>
            </div>
          </DialogBody>
        )}

        <DialogFooter>
          {!subscriptionUrl ? (
            <>
              <Button variant="outline" onClick={close}>
                取消
              </Button>
              <Button
                onClick={handleCreateSubscription}
                disabled={!subscriptionName.trim() || isCreatingSubscription}
              >
                {isCreatingSubscription ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <LinkIcon className="h-4 w-4" />
                )}
                {isEditingExistingSubscription ? "保存更新" : "生成链接"}
              </Button>
            </>
          ) : (
            <Button onClick={close}>完成</Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
