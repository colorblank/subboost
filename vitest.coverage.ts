import { defineConfig, mergeConfig } from "vitest/config";
import baseConfig from "./vitest.config";

const coverageThresholds = {
  lines: 97,
  statements: 97,
  functions: 97,
  branches: 96,
} as const;

export default mergeConfig(
  baseConfig,
  defineConfig({
    test: {
      include: ["test/**/*.test.ts"],
      coverage: {
        provider: "v8",
        include: [
          "packages/**/*.{ts,tsx,js,cjs,mjs}",
          "local/**/*.{ts,tsx,js,cjs,mjs}",
          "scripts/**/*.{ts,tsx,js,cjs,mjs}",
        ],
        exclude: [
          "**/*.{test,spec}.{ts,tsx,js,cjs,mjs}",
          "**/*.test-{helpers,utils}.{ts,tsx,js,cjs,mjs}",
          "**/*.d.ts",
          "**/node_modules/**",
          "**/.next/**",
          "**/out/**",
          "**/dist/**",
          "**/build/**",
          "**/coverage/**",
          "**/.cache/**",
          "**/.turbo/**",
          "**/fixtures/**",
          "**/__fixtures__/**",
          "**/*.config.{ts,tsx,js,cjs,mjs}",
          "**/eslint.config.{ts,js,mjs}",
          "**/next.config.{ts,js,mjs}",
          "**/postcss.config.{ts,js,mjs}",
          "**/tailwind.config.{ts,js,mjs}",
          "local/scripts/build-static.cjs",
        ],
        reporter: ["text", "json-summary", "html", "lcov"],
        reportsDirectory: "coverage",
        thresholds: coverageThresholds,
      },
    },
  }),
);
