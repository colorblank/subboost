import path from "node:path";
import fs from "node:fs";

function loadFrontendConfig() {
  const customPath = process.env.SUBBOOST_FRONTEND_CONFIG;
  const configUrl = customPath ? path.resolve(customPath) : new URL("../config/frontend.json", import.meta.url);
  try {
    return JSON.parse(fs.readFileSync(configUrl, "utf8"));
  } catch {
    return {
      development: { hostname: "127.0.0.1", port: 3001 },
      backendUrl: "http://127.0.0.1:3002",
    };
  }
}

const frontend = loadFrontendConfig();
const isStaticExport = process.env.SUBBOOST_STATIC_EXPORT === "1";

/** @type {import('next').NextConfig} */
const nextConfig = {
  output: isStaticExport ? "export" : "standalone",
  ...(isStaticExport ? { images: { unoptimized: true } } : {}),
  agentRules: false,
  outputFileTracingRoot: path.resolve(process.cwd(), ".."),
  transpilePackages: ["@subboost/core", "@subboost/ui", "@subboost/config"],
  webpack(config) {
    config.resolve.alias["@"] = path.resolve(process.cwd(), "src");
    config.resolve.alias["@local"] = path.resolve(process.cwd(), "src");
    config.resolve.modules = [
      path.resolve(process.cwd(), "node_modules"),
      ...(config.resolve.modules || []),
    ];
    return config;
  },
  ...(isStaticExport
    ? {}
    : {
        async rewrites() {
          const backend = (process.env.SUBBOOST_BACKEND_URL || frontend.backendUrl || "http://127.0.0.1:3002").replace(/\/+$/, "");
          return [{ source: "/api/:path*", destination: `${backend}/api/:path*` }];
        },
        async headers() {
          const headers = [
            { key: "X-Content-Type-Options", value: "nosniff" },
            { key: "X-Frame-Options", value: "DENY" },
            { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
            { key: "Permissions-Policy", value: "camera=(), microphone=(), geolocation=()" },
            { key: "Cache-Control", value: "no-store, no-cache, max-age=0, must-revalidate" },
          ];
          return [
            { source: "/:path*", headers },
          ];
        },
      }),
};

export default nextConfig;
