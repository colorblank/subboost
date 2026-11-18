import path from "path";
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["packages/core/src/**/*.test.ts"],
  },
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "local/src"),
      "@subboost/core": path.resolve(import.meta.dirname, "packages/core/src"),
    },
  },
});
