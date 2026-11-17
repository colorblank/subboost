"use client";

import * as React from "react";
import type { AutoUpdateIntervalPolicyOverride } from "@subboost/core/subscription/auto-update-interval";
import { readJsonResponse } from "@subboost/ui/product/client-response";

export type RuntimeConfig = {
  autoUpdateIntervalPolicy: AutoUpdateIntervalPolicyOverride;
  minimumPasswordCharacters: number;
};

export function useRuntimeConfig() {
  const [config, setConfig] = React.useState<RuntimeConfig | null>(null);
  const [error, setError] = React.useState("");

  React.useEffect(() => {
    const controller = new AbortController();
    fetch("/api/config", { signal: controller.signal, cache: "no-store" })
      .then((response) => readJsonResponse<RuntimeConfig>(response, "读取运行配置失败"))
      .then((config) => {
        if (!controller.signal.aborted) setConfig(config);
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) setError(error instanceof Error ? error.message : "读取运行配置失败");
      });
    return () => controller.abort();
  }, []);

  return { config, error };
}
