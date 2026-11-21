import { createSubBoostFaviconRedirect } from "@subboost/ui/brand-favicon";

export const dynamic = "force-static";

export function GET() {
  return createSubBoostFaviconRedirect();
}
