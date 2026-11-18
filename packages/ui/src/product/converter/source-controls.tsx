"use client";

import { HelpCircle, Menu, Plus } from "lucide-react";
import type { ParsedNode } from "@subboost/core/types/node";
import { Button } from "@subboost/ui/components/ui/button";
import { ChoiceChip, ChoiceGroup } from "@subboost/ui/components/ui/choice-group";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@subboost/ui/components/ui/dropdown-menu";
import {
  Popover,
  PopoverArrow,
  PopoverContent,
  PopoverTrigger,
} from "@subboost/ui/components/ui/popover";
import { cn } from "@subboost/ui/lib/utils";
import { getSubscriptionUserInfoDisplay } from "@subboost/ui/product/subscription/subscription-userinfo-display";
import type { SourceType, SubscriptionSource } from "@subboost/ui/store/config-store";
import { sourceTypeInfo } from "./source-type-info";

const sourceTypes = Object.keys(sourceTypeInfo) as SourceType[];

export function SourceTypeChoices({
  value,
  onChange,
  compact = false,
}: {
  value: SourceType;
  onChange: (type: SourceType) => void;
  compact?: boolean;
}) {
  return (
    <ChoiceGroup label="导入源类型" className={cn("grid w-full grid-cols-3 gap-1 rounded-xl bg-ink/5", compact ? "p-1" : "p-1")}>
      {sourceTypes.map((type) => {
        const info = sourceTypeInfo[type];
        const Icon = info.icon;
        return (
          <ChoiceChip
            key={type}
            selected={value === type}
            onClick={() => onChange(type)}
            title={info.label}
            label={
              <>
                <Icon className="h-3.5 w-3.5" aria-hidden="true" />
                <span className="truncate">{info.label}</span>
              </>
            }
            className={cn(
              "min-h-9 min-w-0 gap-1.5 rounded-lg border-0 px-2 py-1.5 text-xs font-medium shadow-none",
              value === type
                ? "bg-surface-raised text-indigo-300 shadow-sm"
                : "bg-transparent text-fg-60 hover:bg-ink/5 hover:text-fg"
            )}
          />
        );
      })}
    </ChoiceGroup>
  );
}

export function SourceStatusPopover({
  source,
  nodes,
}: {
  source: SubscriptionSource;
  nodes: ParsedNode[];
}) {
  if (!source.parsed || source.nodeCount === undefined) return null;

  const userInfo = getSubscriptionUserInfoDisplay(source.subscriptionUserInfo, nodes);
  const hasUserInfo = Boolean(userInfo && (userInfo.traffic || userInfo.expire));

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-7 gap-1.5 whitespace-nowrap rounded-lg border border-green-500/20 bg-green-500/10 px-2 text-xs font-medium text-green-300 hover:bg-green-500/15"
          title="查看流量/到期"
          aria-label="查看流量/到期"
        >
          ✓ {source.nodeCount} 节点
          <Menu className="h-3 w-3 text-green-300/70" aria-hidden="true" />
        </Button>
      </PopoverTrigger>
      <PopoverContent side="bottom" align="start" className="w-[260px] bg-shade/90 p-3">
        <div className="space-y-2 text-xs">
          <div className="flex items-center gap-2">
            <HelpCircle className="h-4 w-4 text-green-300" aria-hidden="true" />
            <p className="font-medium text-fg">订阅信息</p>
          </div>
          {hasUserInfo && userInfo ? (
            <div className="space-y-1 text-fg-60">
              {userInfo.traffic ? <p>已用流量：{userInfo.traffic}</p> : null}
              {userInfo.expire ? <p>到期时间：{userInfo.expire}</p> : null}
            </div>
          ) : (
            <p className="leading-relaxed text-fg-60">暂无已用流量/到期时间信息</p>
          )}
        </div>
        <PopoverArrow className="fill-shade/90" />
      </PopoverContent>
    </Popover>
  );
}

export function AddSourceMenu({
  open,
  onOpenChange,
  onAdd,
  compact = false,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAdd: (type: SourceType) => void;
  compact?: boolean;
}) {
  return (
    <DropdownMenu open={open} onOpenChange={onOpenChange}>
      <DropdownMenuTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          className="h-10 w-full rounded-xl border-dashed border-ink/15 text-sm text-fg-60 hover:border-indigo-500/40 hover:bg-indigo-500/5 hover:text-fg"
        >
          <Plus className="h-3.5 w-3.5" aria-hidden="true" />
          添加订阅/节点源
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-[var(--radix-dropdown-menu-trigger-width)] border-ink/10 bg-surface-raised text-fg">
        {sourceTypes.map((type) => {
          const info = sourceTypeInfo[type];
          const Icon = info.icon;
          return (
            <DropdownMenuItem
              key={type}
              onSelect={() => onAdd(type)}
              className={cn("gap-3 focus:bg-ink/5 focus:text-fg", compact ? "py-2" : "py-2.5")}
            >
              <Icon className="h-4 w-4 shrink-0 text-indigo-400" aria-hidden="true" />
              <span>
                <span className="block text-sm font-medium text-fg">{info.label}</span>
                {!compact ? <span className="block text-xs text-fg-45">{info.description}</span> : null}
              </span>
            </DropdownMenuItem>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
