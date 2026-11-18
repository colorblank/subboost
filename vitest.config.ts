import { defineConfig } from "vitest/config";
import path from "node:path";
import { transformWithOxc } from "vite";

function tsxTransformPlugin() {
  return {
    name: "subboost-public-vitest-tsx-transform",
    enforce: "pre" as const,
    async transform(code: string, id: string) {
      const filePath = id.split("?")[0].replace(/\\/g, "/");
      if (!filePath.endsWith(".tsx")) return null;

      return transformWithOxc(code, id, {
        jsx: {
          runtime: "automatic",
          importSource: "react",
        },
        sourcemap: true,
      });
    },
  };
}

export default defineConfig({
  plugins: [tsxTransformPlugin()],
  test: {
    environment: "node",
    setupFiles: ["./test/setup-runtime.ts"],
    include: [
      "packages/core/src/**/*.test.ts",
      "packages/ui/src/**/*.test.ts",
      "local/src/**/*.test.ts",
      "local/scripts/**/*.test.ts",
      "test/regression/tooling/**/*.test.ts",
    ],
    restoreMocks: true,
  },
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "local/src"),
      "@local": path.resolve(import.meta.dirname, "local/src"),
      "@subboost/core": path.resolve(import.meta.dirname, "packages/core/src"),
      "@subboost/ui": path.resolve(import.meta.dirname, "packages/ui/src"),
      "@subboost/config": path.resolve(import.meta.dirname, "packages/config"),
    },
  },
});
