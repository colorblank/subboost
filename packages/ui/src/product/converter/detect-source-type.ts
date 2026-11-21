import type { SourceType } from "@subboost/ui/store/config-store";

const NODE_LINK_SCHEME_RE = /^(?:ss|ssr|vmess|vless|trojan|hysteria2?|hy2|tuic|wireguard|anytls|snell|socks5?):\/\//i;
const HTTP_URL_RE = /^https?:\/\/[^\s]+$/i;
const YAML_PROXIES_RE = /(?:^|\n)\s*proxies\s*:/;

export function detectSourceTypeFromContent(content: string): SourceType | null {
  const trimmed = content.trim();
  if (!trimmed) return null;

  if (HTTP_URL_RE.test(trimmed)) return "url";
  if (YAML_PROXIES_RE.test(trimmed)) return "yaml";

  const firstNonEmptyLine = trimmed.split(/\r?\n/).find((line) => line.trim().length > 0)?.trim() ?? "";
  if (NODE_LINK_SCHEME_RE.test(firstNonEmptyLine)) return "nodes";

  return null;
}
