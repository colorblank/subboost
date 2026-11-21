"use client";

import { FormField } from "@subboost/ui/components/ui/form-field";
import { Input } from "@subboost/ui/components/ui/input";
import { Switch } from "@subboost/ui/components/ui/switch";
import { SwitchField } from "@subboost/ui/components/ui/switch-field";
import { SmartNodeMatchingHelp } from "@subboost/ui/components/subscription/smart-node-matching-help";
import type { AutoUpdateIntervalPolicy } from "@subboost/core/subscription/auto-update-interval";

type SubscriptionSettingsFieldsProps = {
  name: string;
  onNameChange: (value: string) => void;
  smartNodeMatchingEnabled: boolean;
  onSmartNodeMatchingChange: (value: boolean) => void;
  autoUpdateEnabled: boolean;
  onAutoUpdateEnabledChange: (value: boolean) => void;
  autoUpdateHours: number;
  onAutoUpdateHoursChange: (value: number) => void;
  autoUpdatePolicy: AutoUpdateIntervalPolicy;
  disabled?: boolean;
  autoUpdateDescription?: string;
  autoUpdateHoursDescription?: string;
};

export function SubscriptionSettingsFields({
  name,
  onNameChange,
  smartNodeMatchingEnabled,
  onSmartNodeMatchingChange,
  autoUpdateEnabled,
  onAutoUpdateEnabledChange,
  autoUpdateHours,
  onAutoUpdateHoursChange,
  autoUpdatePolicy,
  disabled,
  autoUpdateDescription,
  autoUpdateHoursDescription,
}: SubscriptionSettingsFieldsProps) {
  return (
    <>
      <FormField label="订阅名称">
        <Input
          placeholder="例如：我的配置"
          value={name}
          onChange={(e) => onNameChange(e.target.value)}
          maxLength={100}
          disabled={disabled}
        />
      </FormField>

      <div className="rounded-xl border border-ink/10 bg-ink/5 p-3.5 space-y-3">
        <div className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <p className="text-sm font-medium text-fg-80">更新时智能匹配节点</p>
              <SmartNodeMatchingHelp enabled={smartNodeMatchingEnabled} />
            </div>
          </div>
          <Switch
            checked={smartNodeMatchingEnabled}
            onCheckedChange={onSmartNodeMatchingChange}
            aria-label="更新时智能匹配节点"
            disabled={disabled}
          />
        </div>

        <div className="border-t border-ink/10" />

        <SwitchField
          label="启用自动更新"
          description={autoUpdateDescription}
          appearance="plain"
          checked={autoUpdateEnabled}
          onCheckedChange={onAutoUpdateEnabledChange}
          disabled={disabled}
        />

        {autoUpdateEnabled && (
          <FormField
            label="自动更新间隔（小时）"
            description={autoUpdateHoursDescription}
            className="pt-1"
          >
            <Input
              type="number"
              min={autoUpdatePolicy.minHours}
              step={autoUpdatePolicy.stepHours}
              value={autoUpdateHours}
              onChange={(e) => onAutoUpdateHoursChange(Number(e.target.value))}
              disabled={disabled}
            />
          </FormField>
        )}
      </div>
    </>
  );
}
