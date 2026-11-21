"use client";

import { Check, ChevronDown, ChevronUp, Loader2, Maximize2, X } from "lucide-react";
import type { ParsedNode } from "@subboost/core/types/node";
import { Button } from "@subboost/ui/components/ui/button";
import { IconButton } from "@subboost/ui/components/ui/icon-button";
import { Input } from "@subboost/ui/components/ui/input";
import { Textarea } from "@subboost/ui/components/ui/textarea";
import type { SourceType, SubscriptionSource } from "@subboost/ui/store/config-store";
import { buildSourceDisplayLabel } from "@subboost/ui/product/converter/source-display-label";
import { SourceStatusPopover, SourceTypeChoices } from "@subboost/ui/product/converter/source-controls";
import { sourceTypeInfo } from "@subboost/ui/product/converter/source-type-info";
import { SubscriptionImportErrorBadge } from "@subboost/ui/product/converter/subscription-import-error";

type SourceCardsProps = {
  sources: SubscriptionSource[];
  nodesBySourceId: Map<string, ParsedNode[]>;
  compact?: boolean;
  cardClassName?: string;
  onMoveSource: (id: string, direction: "up" | "down") => void;
  onExpandSource: (id: string) => void;
  onImportSource: (id: string) => void;
  onRemoveSource: (id: string) => void;
  onUpdateSourceType: (id: string, type: SourceType) => void;
  onUpdateSourceContent: (id: string, content: string) => void;
};

export function SourceCards({
  sources,
  nodesBySourceId,
  compact = false,
  cardClassName = "space-y-3 rounded-xl border border-ink/10 p-3 transition-colors focus-within:border-indigo-500/40",
  onMoveSource,
  onExpandSource,
  onImportSource,
  onRemoveSource,
  onUpdateSourceType,
  onUpdateSourceContent,
}: SourceCardsProps) {
  return (
    <>
      {sources.map((source, index) => {
        const typeInfo = sourceTypeInfo[source.type];
        const sourceDisplayLabel = buildSourceDisplayLabel({
          typeLabel: typeInfo.label,
          tag: source.tag,
          order: index + 1,
          total: sources.length,
        });
        const sourceNodes = nodesBySourceId.get(source.id) ?? [];

        return (
          <div key={source.id} className={cardClassName}>
            <div className="flex flex-wrap items-center justify-between gap-1.5">
              <div className="flex min-w-0 flex-wrap items-center gap-1.5">
                <span className="whitespace-nowrap text-sm font-medium text-fg-80">
                  {sourceDisplayLabel}
                </span>
                <SourceStatusPopover source={source} nodes={sourceNodes} />
                {(source.errorInfo || source.error) && (
                  <SubscriptionImportErrorBadge errorInfo={source.errorInfo} errorMessage={source.error} />
                )}
              </div>
              <div className="ml-auto flex shrink-0 items-center gap-0.5">
                <div className="flex items-center">
                  <IconButton
                    label="上移"
                    variant="ghost"
                    onClick={() => onMoveSource(source.id, "up")}
                    disabled={index <= 0}
                    className="h-7 w-7 rounded-lg p-1 text-fg-50 hover:bg-ink/5 hover:text-fg"
                  >
                    <ChevronUp className="h-3.5 w-3.5" aria-hidden="true" />
                  </IconButton>
                  <IconButton
                    label="下移"
                    variant="ghost"
                    onClick={() => onMoveSource(source.id, "down")}
                    disabled={index >= sources.length - 1}
                    className="h-7 w-7 rounded-lg p-1 text-fg-50 hover:bg-ink/5 hover:text-fg"
                  >
                    <ChevronDown className="h-3.5 w-3.5" aria-hidden="true" />
                  </IconButton>
                </div>
                <IconButton
                  label="高级编辑"
                  variant="ghost"
                  onClick={() => onExpandSource(source.id)}
                  className="h-7 w-7 rounded-lg p-1 text-fg-50 hover:bg-ink/5 hover:text-fg"
                >
                  <Maximize2 className="h-3.5 w-3.5" aria-hidden="true" />
                </IconButton>
                <Button
                  type="button"
                  size="sm"
                  aria-label={source.parsing ? "导入中" : source.parsed ? "重新导入" : "导入此源"}
                  variant={source.parsed ? "secondary" : "default"}
                  onClick={() => onImportSource(source.id)}
                  disabled={!source.content.trim() || source.parsing}
                  className="h-7 whitespace-nowrap rounded-lg px-2.5 text-xs"
                  title={source.parsing ? "导入中..." : source.parsed ? "重新导入" : "导入此源"}
                >
                  {source.parsing ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
                  ) : source.parsed ? (
                    <Check className="h-3.5 w-3.5" aria-hidden="true" />
                  ) : null}
                  <span>{source.parsing ? "导入中" : source.parsed ? "重新导入" : "导入"}</span>
                </Button>
                {sources.length > 1 && (
                  <IconButton
                    label="删除导入源"
                    variant="ghost"
                    onClick={() => onRemoveSource(source.id)}
                    className="h-7 w-7 rounded-lg p-1 text-fg-50 hover:bg-red-500/10 hover:text-red-400"
                  >
                    <X className="h-3.5 w-3.5" aria-hidden="true" />
                  </IconButton>
                )}
              </div>
            </div>

            <SourceTypeChoices
              value={source.type}
              onChange={(type) => onUpdateSourceType(source.id, type)}
              compact={compact}
            />

            {source.type === "url" ? (
              <Input
                aria-label={`${sourceDisplayLabel}订阅链接`}
                value={source.content}
                onChange={(e) => onUpdateSourceContent(source.id, e.target.value)}
                placeholder={typeInfo.placeholder}
                className="h-10 text-sm"
              />
            ) : (
              <Textarea
                aria-label={`${sourceDisplayLabel}节点内容`}
                value={source.content}
                onChange={(e) => onUpdateSourceContent(source.id, e.target.value)}
                placeholder={typeInfo.placeholder}
                className="min-h-28 font-mono text-sm leading-6"
              />
            )}
          </div>
        );
      })}
    </>
  );
}
