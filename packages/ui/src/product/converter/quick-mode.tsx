"use client";

import { SourcesSection } from "./quick-mode/sources-section";
import { TemplatesSection, type QuickTemplateUploadEntry } from "./quick-mode/templates-section";

type Props = {
  templateUpload?: QuickTemplateUploadEntry | null;
};

export function QuickMode({ templateUpload }: Props) {
  return (
    <div className="flex flex-col gap-3">
      <section aria-labelledby="import-step" className="space-y-3">
        <h2 id="import-step" className="text-sm font-semibold text-fg-70">1 · 导入订阅或节点</h2>
        <SourcesSection />
      </section>
      <section aria-labelledby="configure-step" className="space-y-3 border-t border-ink/10 pt-3">
        <h2 id="configure-step" className="text-sm font-semibold text-fg-70">2 · 选择配置模板</h2>
        <TemplatesSection templateUpload={templateUpload} />
      </section>
    </div>
  );
}
