"use client";

import { AlertCircle, Server } from "lucide-react";
import { Badge } from "@subboost/ui/components/ui/badge";
import { AddSourceMenu } from "@subboost/ui/product/converter/source-controls";
import { SourceCards } from "@subboost/ui/product/converter/source-cards";
import { SourceEditorDialog } from "@subboost/ui/product/converter/source-editor-dialog";
import { useSubscriptionSourcesController } from "@subboost/ui/product/converter/use-subscription-sources-controller";
import { SectionHeader } from "../section-header";

export function InputSection({
  isExpanded,
  onToggle,
}: {
  isExpanded: boolean;
  onToggle: () => void;
}) {
  const {
    addSource,
    applyExpandedSourceContent,
    closeExpandedSourceEditor,
    expandedSourceTailscaleStateDirs,
    error: globalError,
    expandedSource,
    expandedSourcePreviewName,
    handleImportSource,
    moveSource,
    nodesBySourceId,
    removeSource,
    setExpandedSourceId,
    setShowAddMenu,
    showAddMenu,
    sources,
    updateSource,
    updateSourceMeta,
    updateSourceType,
  } = useSubscriptionSourcesController({ mode: "advanced" });
  const sourceCount = sources.length;

  return (
    <div>
      <SectionHeader
        icon={Server}
        title="节点导入"
        isExpanded={isExpanded}
        onToggle={onToggle}
        badge={
          sourceCount > 0 && (
            <Badge variant="outline" className="ml-auto border-blue-500/50 bg-blue-500/10 text-blue-300">
              {sourceCount} 个导入源
            </Badge>
          )
        }
      />

      {isExpanded && (
        <div className="mt-2.5 space-y-3">
          <SourceCards
            sources={sources}
            nodesBySourceId={nodesBySourceId}
            compact
            onMoveSource={moveSource}
            onExpandSource={setExpandedSourceId}
            onImportSource={(id) => void handleImportSource(id)}
            onRemoveSource={removeSource}
            onUpdateSourceType={updateSourceType}
            onUpdateSourceContent={updateSource}
          />
          {globalError && (
            <div className="flex items-center gap-2 text-xs text-red-400">
              <AlertCircle className="h-3.5 w-3.5" />
              <span className="break-all">{globalError}</span>
            </div>
          )}
          <AddSourceMenu open={showAddMenu} onOpenChange={setShowAddMenu} onAdd={addSource} compact />
        </div>
      )}

      <SourceEditorDialog
        source={expandedSource}
        previewName={expandedSourcePreviewName}
        onClose={closeExpandedSourceEditor}
        onUpdateContent={updateSource}
        onUpdateMeta={updateSourceMeta}
        onApplyContent={applyExpandedSourceContent}
        tailscaleStateDirs={expandedSourceTailscaleStateDirs}
      />
    </div>
  );
}
