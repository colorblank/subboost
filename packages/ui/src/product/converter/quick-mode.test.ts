import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

vi.mock("./quick-mode/sources-section", () => ({
  SourcesSection: () => React.createElement("div", null, "sources"),
}));
vi.mock("./quick-mode/templates-section", () => ({
  TemplatesSection: () => React.createElement("div", null, "templates"),
}));

import { QuickMode } from "./quick-mode";

describe("QuickMode", () => {
  it("renders with and without advanced-mode guidance", () => {
    const basicHtml = renderToStaticMarkup(React.createElement(QuickMode, {}));
    expect(basicHtml).toContain("1 · 导入订阅或节点");
    expect(basicHtml).not.toContain("切换到高级模式");

    const onSwitchToAdvanced = vi.fn();
    const guidedHtml = renderToStaticMarkup(
      React.createElement(QuickMode, { onSwitchToAdvanced })
    );
    expect(guidedHtml).toContain("切换到高级模式");
  });
});
