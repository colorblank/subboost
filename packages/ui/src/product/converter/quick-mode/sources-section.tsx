import { X, AlertCircle, Check, Loader2, Maximize2, ChevronUp, ChevronDown } from "lucide-react";
import { Button } from "@subboost/ui/components/ui/button";
import { IconButton } from "@subboost/ui/components/ui/icon-button";
import { Textarea } from "@subboost/ui/components/ui/textarea";
import { Input } from "@subboost/ui/components/ui/input";
import { Badge } from "@subboost/ui/components/ui/badge";
import { SubscriptionImportErrorBadge } from "@subboost/ui/product/converter/subscription-import-error";
import { buildSourceDisplayLabel } from "@subboost/ui/product/converter/source-display-label";
import { AddSourceMenu, SourceStatusPopover, SourceTypeChoices } from "@subboost/ui/product/converter/source-controls";
import { SourceEditorDialog } from "@subboost/ui/product/converter/source-editor-dialog";
import { useSubscriptionSourcesController } from "@subboost/ui/product/converter/use-subscription-sources-controller";
import { sourceTypeInfo } from "./constants";

export function SourcesSection() {
  const {
    addSource,
    applyExpandedSourceContent,
    closeExpandedSourceEditor,
    expandedSourceTailscaleStateDirs,
    error,
    expandedSource,
    expandedSourcePreviewName,
    handleImportSource,
    moveSource,
    nodeCount,
    nodesBySourceId,
    removeSource,
    setExpandedSourceId,
    setShowAddMenu,
    showAddMenu,
    sources,
    updateSource,
    updateSourceMeta,
    updateSourceType,
  } = useSubscriptionSourcesController({ mode: "quick" });

  return (
    <>
      {/* Sources List */}
      <div className="flex flex-col gap-3">
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
            <div key={source.id} className="flex shrink-0 flex-col gap-3 rounded-xl border border-ink/10 p-3 transition-colors focus-within:border-indigo-500/40">
              {/* Source Header */}
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex min-w-0 flex-wrap items-center gap-2">
                  <span className="text-sm font-medium text-fg-80">
                    {sourceDisplayLabel}
                  </span>
                  <SourceStatusPopover source={source} nodes={sourceNodes} />
                  {(source.errorInfo || source.error) && (
                    <SubscriptionImportErrorBadge errorInfo={source.errorInfo} errorMessage={source.error} />
                  )}
                </div>
                <div className="ml-auto flex shrink-0 items-center gap-1">
                  <div className="flex items-center">
                    <IconButton
                      label="上移"
                      variant="ghost"
                      onClick={() => moveSource(source.id, "up")}
                      disabled={index <= 0}
                      className="h-8 w-8 rounded-lg p-1 text-fg-50 hover:bg-ink/5 hover:text-fg"
                    >
                      <ChevronUp className="h-3.5 w-3.5" aria-hidden="true" />
                    </IconButton>
                    <IconButton
                      label="下移"
                      variant="ghost"
                      onClick={() => moveSource(source.id, "down")}
                      disabled={index >= sources.length - 1}
                      className="h-8 w-8 rounded-lg p-1 text-fg-50 hover:bg-ink/5 hover:text-fg"
                    >
                      <ChevronDown className="h-3.5 w-3.5" aria-hidden="true" />
                    </IconButton>
                  </div>
                  <IconButton
                    label="高级编辑"
                    variant="ghost"
                    onClick={() => setExpandedSourceId(source.id)}
                    className="h-8 w-8 rounded-lg p-1 text-fg-50 hover:bg-ink/5 hover:text-fg"
                  >
                    <Maximize2 className="h-3.5 w-3.5" aria-hidden="true" />
                  </IconButton>
                  <Button
                    type="button"
                    size="sm"
                    aria-label={source.parsing ? "导入中" : source.parsed ? "重新导入" : "导入此源"}
                    variant={source.parsed ? "secondary" : "default"}
                    onClick={() => void handleImportSource(source.id)}
                    disabled={!source.content.trim() || source.parsing}
                    className="h-8 w-24 rounded-lg px-3"
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
                      onClick={() => removeSource(source.id)}
                      className="h-8 w-8 rounded-lg p-1 text-fg-50 hover:bg-red-500/10 hover:text-red-400"
                    >
                      <X className="h-3.5 w-3.5" aria-hidden="true" />
                    </IconButton>
                  )}
                </div>
              </div>
              <SourceTypeChoices value={source.type} onChange={(type) => updateSourceType(source.id, type)} />

              {/* Source Input */}
              {source.type === "url" ? (
                <Input
                  aria-label={`${sourceDisplayLabel}订阅链接`}
                  value={source.content}
                  onChange={(e) => updateSource(source.id, e.target.value)}
                  placeholder={typeInfo.placeholder}
                  className="h-10 text-sm"
                />
              ) : (
                <Textarea
                  aria-label={`${sourceDisplayLabel}节点内容`}
                  value={source.content}
                  onChange={(e) => updateSource(source.id, e.target.value)}
                  placeholder={typeInfo.placeholder}
                  className="min-h-28 font-mono text-sm leading-6"
                />
              )}
            </div>
          );
        })}

        <div className="flex-shrink-0">
          <AddSourceMenu open={showAddMenu} onOpenChange={setShowAddMenu} onAdd={addSource} />
        </div>

        {/* Status */}
        {error && (
          <div className="flex items-center gap-2 text-xs text-red-400 flex-shrink-0">
            <AlertCircle className="h-3.5 w-3.5" />
            {error}
          </div>
        )}
        {nodeCount > 0 && (
          <div className="flex items-center gap-2 flex-shrink-0">
            <Badge variant="success">{nodeCount} 个节点已解析</Badge>
          </div>
        )}
      </div>

      <SourceEditorDialog
        source={expandedSource}
        previewName={expandedSourcePreviewName}
        onClose={closeExpandedSourceEditor}
        onUpdateContent={updateSource}
        onUpdateMeta={updateSourceMeta}
        onApplyContent={applyExpandedSourceContent}
        tailscaleStateDirs={expandedSourceTailscaleStateDirs}
      />
    </>
  );
}
