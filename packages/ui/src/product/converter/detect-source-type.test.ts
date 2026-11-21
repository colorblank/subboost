import { describe, expect, it } from "vitest";
import { detectSourceTypeFromContent } from "./detect-source-type";

describe("detectSourceTypeFromContent", () => {
  it("detects subscription URLs", () => {
    expect(detectSourceTypeFromContent("https://example.com/sub?token=abc")).toBe("url");
    expect(detectSourceTypeFromContent("  http://127.0.0.1:8080/sub  ")).toBe("url");
  });

  it("detects Clash/Mihomo YAML configs", () => {
    expect(detectSourceTypeFromContent("proxies:\n  - name: HK\n    type: ss")).toBe("yaml");
    expect(detectSourceTypeFromContent("mixed-port: 7890\nproxies:\n  - name: JP")).toBe("yaml");
  });

  it("detects proxy node links", () => {
    expect(detectSourceTypeFromContent("vless://uuid@example.com:443#HK")).toBe("nodes");
    expect(detectSourceTypeFromContent("hysteria2://pass@example.com:443\nss://abc@1.2.3.4:8388")).toBe("nodes");
  });

  it("returns null for empty or ambiguous text", () => {
    expect(detectSourceTypeFromContent("")).toBeNull();
    expect(detectSourceTypeFromContent("   ")).toBeNull();
    expect(detectSourceTypeFromContent("hello world")).toBeNull();
  });
});
