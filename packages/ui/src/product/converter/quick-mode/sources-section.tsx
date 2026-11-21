import { AlertCircle } from "lucide-react";
import { Badge } from "@subboost/ui/components/ui/badge";
import { AddSourceMenu } from "@subboost/ui/product/converter/source-controls";
import { SourceCards } from "@subboost/ui/product/converter/source-cards";
import { SourceEditorDialog } from "@subboost/ui/product/converter/source-editor-dialog";
import { useSubscriptionSourcesController } from "@subboost/ui/product/converter/use-subscription-sources-controller";

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
      <div className="flex flex-col gap-3">
        <SourceCards
          sources={sources}
          nodesBySourceId={nodesBySourceId}
          cardClassName="flex shrink-0 flex-col gap-3 rounded-xl border border-ink/10 p-3 transition-colors focus-within:border-indigo-500/40"
          onMoveSource={moveSource}
          onExpandSource={setExpandedSourceId}
          onImportSource={(id) => void handleImportSource(id)}
          onRemoveSource={removeSource}
          onUpdateSourceType={updateSourceType}
          onUpdateSourceContent={updateSource}
        />

        <div className="flex-shrink-0">
          <AddSourceMenu open={showAddMenu} onOpenChange={setShowAddMenu} onAdd={addSource} />
        </div>

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
