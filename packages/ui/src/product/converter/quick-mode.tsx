"use client";

import { ArrowRight, Settings2 } from "lucide-react";
import { Button } from "@subboost/ui/components/ui/button";
import { SourcesSection } from "./quick-mode/sources-section";
import { TemplatesSection, type QuickTemplateUploadEntry } from "./quick-mode/templates-section";

type Props = {
  templateUpload?: QuickTemplateUploadEntry | null;
  onSwitchToAdvanced?: () => void;
};

export function QuickMode({ templateUpload, onSwitchToAdvanced }: Props) {
  return (
    <div className="flex flex-col gap-5">
      <section aria-labelledby="import-step" className="space-y-3">
        <h2 id="import-step" className="text-sm font-semibold text-fg-80">1 · 导入订阅或节点</h2>
        <SourcesSection />
      </section>
      <section aria-labelledby="configure-step" className="space-y-3 border-t border-ink/10 pt-4">
        <h2 id="configure-step" className="text-sm font-semibold text-fg-80">2 · 选择配置模板</h2>
        <TemplatesSection templateUpload={templateUpload} />
      </section>
      {onSwitchToAdvanced && (
        <div className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-ink/10 bg-ink/[0.03] px-3.5 py-2.5 text-xs text-fg-60">
          <div className="flex items-center gap-2 min-w-0">
            <Settings2 className="h-3.5 w-3.5 shrink-0 text-indigo-400" aria-hidden="true" />
            <span>需要过滤信息节点、批量改名、配置链式代理或自定义分流？</span>
          </div>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={onSwitchToAdvanced}
            className="h-7 shrink-0 gap-1 px-2 text-xs font-medium text-indigo-400 hover:bg-indigo-500/10 hover:text-indigo-300"
          >
            <span>切换到高级模式</span>
            <ArrowRight className="h-3.5 w-3.5" aria-hidden="true" />
          </Button>
        </div>
      )}
    </div>
  );
}
