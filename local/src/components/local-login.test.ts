import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
vi.mock("@local/lib/runtime-config", () => import("../../../test/runtime-config-fixture"));

const mocks = vi.hoisted(() => ({
  forms: [] as any[],
  inputs: [] as any[],
  buttons: [] as any[],
}));

const stateMock = vi.hoisted(() => ({
  enabled: false,
  callIndex: 0,
  overrides: {} as Record<number, unknown>,
  runEffects: false,
  setters: [] as Array<ReturnType<typeof vi.fn>>,
}));

vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  return {
    ...actual,
    useEffect: (effect: React.EffectCallback, deps?: React.DependencyList) => {
      if (stateMock.enabled && stateMock.runEffects) {
        effect();
        return;
      }
      return actual.useEffect(effect, deps);
    },
    useState: (initial: unknown) => {
      if (!stateMock.enabled) return actual.useState(initial);
      const index = stateMock.callIndex++;
      const value = Object.prototype.hasOwnProperty.call(stateMock.overrides, index)
        ? stateMock.overrides[index]
        : initial;
      const setter = vi.fn((next: unknown) => {
        const resolved =
          typeof next === "function" ? (next as (prev: unknown) => unknown)(value) : next;
        (setter as any).lastValue = resolved;
        return resolved;
      });
      stateMock.setters[index] = setter;
      return [value, setter];
    },
  };
});

vi.mock("react/jsx-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-runtime")>();
  const capture = (type: any, props: any) => {
    if (!stateMock.enabled) return;
    if (type === "form") mocks.forms.push(props ?? {});
    if (type === "input") mocks.inputs.push(props ?? {});
    if (type === "button") mocks.buttons.push(props ?? {});
  };
  return {
    ...actual,
    jsx: (type: any, props: any, key: any) => {
      capture(type, props);
      return actual.jsx(type, props, key);
    },
    jsxs: (type: any, props: any, key: any) => {
      capture(type, props);
      return actual.jsxs(type, props, key);
    },
  };
});

vi.mock("next/image", () => ({
  default: ({ src, alt, width, height, ...props }: React.ImgHTMLAttributes<HTMLImageElement>) =>
    React.createElement("img", { src, alt, width, height, ...props }),
}));
vi.mock("next/link", () => ({
  default: ({ href, children, ...props }: React.PropsWithChildren<{ href: string }>) =>
    React.createElement("a", { href, ...props }, children),
}));
vi.mock("lucide-react", () => ({
  Eye: () => React.createElement("span", null, "eye-icon"),
  EyeOff: () => React.createElement("span", null, "eye-off-icon"),
  Loader2: () => React.createElement("span", null, "loader-icon"),
}));
vi.mock("@subboost/ui/store/config-store/auth-handoff", () => ({
  hasAuthConfigHandoff: vi.fn(() => false),
}));

import { LocalLogin } from "./local-login";
import { hasAuthConfigHandoff } from "@subboost/ui/store/config-store/auth-handoff";

function response(body: unknown, ok = true) {
  return {
    ok,
    text: vi.fn(async () => (body === "" ? "" : JSON.stringify(body))),
  } as unknown as Response;
}

function installWindow(hash = "") {
  const replaceState = vi.fn();
  Object.defineProperty(globalThis, "window", {
    value: {
      location: { href: "", hash, pathname: "/login", search: "" },
      history: { replaceState },
    },
    configurable: true,
  });
  return (globalThis as any).window as {
    location: { href: string; hash: string; pathname: string; search: string };
    history: { replaceState: ReturnType<typeof vi.fn> };
  };
}

function renderLogin(overrides: Record<number, unknown> = {}, runEffects = false) {
  stateMock.enabled = true;
  stateMock.callIndex = 0;
  stateMock.overrides = overrides;
  stateMock.runEffects = runEffects;
  stateMock.setters = [];
  mocks.forms = [];
  mocks.inputs = [];
  mocks.buttons = [];
  try {
    const html = renderToStaticMarkup(React.createElement(LocalLogin));
    return { html, setters: stateMock.setters };
  } finally {
    stateMock.enabled = false;
    stateMock.runEffects = false;
  }
}

async function flushPromises() {
  for (let index = 0; index < 6; index += 1) {
    await Promise.resolve();
  }
}

describe("local login component", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(hasAuthConfigHandoff).mockReturnValue(false);
    delete (globalThis as any).fetch;
    delete (globalThis as any).window;
  });

  it("renders the initial loading card without requiring browser fetch", () => {
    const { html } = renderLogin();

    expect(html).toContain("欢迎使用 SubBoost");
    expect(html).toContain("href=\"/\"");
    expect(html).toContain("src=\"/logo.png\"");
    expect(html).toContain("animate-pulse");
  });
  it("shows failed auth loading and lets the user reload instead of staying on the skeleton", () => {
    const reload = vi.fn();
    vi.stubGlobal("window", { location: { reload } });
    const { html } = renderLogin({ 0: null, 5: "认证服务暂不可用" });
    expect(html).toContain("认证服务暂不可用");
    expect(html).toContain('role="alert"');
    expect(html).not.toContain("animate-pulse");
    mocks.buttons.find((button) => button.children === "重新加载").onClick();
    expect(reload).toHaveBeenCalledOnce();
  });


  it("loads auth state and redirects authenticated users", async () => {
    const win = installWindow();
    (globalThis as any).fetch = vi.fn(async () => response({ setupRequired: false, authenticated: true }));
    vi.mocked(hasAuthConfigHandoff).mockReturnValue(true);

    const { setters } = renderLogin({}, true);
    await flushPromises();

    expect(globalThis.fetch).toHaveBeenCalledWith("/api/auth/me", { cache: "no-store" });
    expect(setters[0]).toHaveBeenCalledWith({ setupRequired: false, authenticated: true });
    expect(win.location.href).toBe("/");
  });

  it("keeps optional admin login available to automatically initialized guests", async () => {
    const win = installWindow();
    const guest = { setupRequired: false, authenticated: true, isGuest: true, adminSetupRequired: false };
    vi.stubGlobal("fetch", vi.fn(async () => response(guest)));
    const { setters } = renderLogin({}, true);
    await flushPromises();
    expect(setters[0]).toHaveBeenCalledWith(guest);
    expect(win.location.href).toBe("");
    renderLogin({ 0: guest });
    expect(mocks.forms).toHaveLength(1);
    expect(mocks.inputs.find((input) => input.autoComplete === "username")).toBeDefined();
  });

  it("offers first-admin setup without requiring guest setup", async () => {
    installWindow("#setup-token=setup-secret");
    vi.stubGlobal("fetch", vi.fn(async () => response({ success: true })));
    renderLogin({
      0: { setupRequired: false, authenticated: true, isGuest: true, adminSetupRequired: true },
      1: "admin",
      2: "long-secret",
      3: "long-secret",
      6: "setup-secret",
    });
    await mocks.forms[0].onSubmit({ preventDefault: vi.fn() });
    expect(globalThis.fetch).toHaveBeenCalledWith("/api/setup/admin", expect.objectContaining({ method: "POST" }));
  });

  it("shows setup mismatch errors before sending a request", async () => {
    installWindow();
    (globalThis as any).fetch = vi.fn();
    const { html, setters } = renderLogin({
      0: { setupRequired: true, authenticated: false },
      1: "admin",
      2: "secret",
      3: "different",
    });

    expect(html).toContain("初始化本地管理员账号");
    expect(html).toContain("创建管理员");
    expect(html).toContain("至少 10 个字符");
    expect(mocks.inputs.find((input) => input.placeholder === "确认密码")).toEqual(
      expect.objectContaining({ autoComplete: "new-password" }),
    );

    await mocks.forms[0].onSubmit({ preventDefault: vi.fn() });

    expect(setters[5]).toHaveBeenCalledWith("密码至少需要 10 个字符");
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  it("shows setup password mismatch errors before sending a request", async () => {
    installWindow();
    (globalThis as any).fetch = vi.fn();
    const { setters } = renderLogin({
      0: { setupRequired: true, authenticated: false },
      1: "admin",
      2: "long-password",
      3: "different-password",
    });

    await mocks.forms[0].onSubmit({ preventDefault: vi.fn() });

    expect(setters[5]).toHaveBeenCalledWith("两次输入的密码不一致，请重新确认");
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  it("logs in existing local admins and toggles password visibility", async () => {
    const win = installWindow();
    (globalThis as any).fetch = vi.fn(async () => response(""));
    const { setters } = renderLogin({
      0: { setupRequired: false, authenticated: false },
      1: "admin",
      2: "secret",
    });

    mocks.inputs.find((input) => input.placeholder === "管理员账号").onChange({ target: { value: "root" } });
    mocks.inputs.find((input) => input.placeholder === "密码").onChange({ target: { value: "next" } });
    mocks.buttons.find((button) => button.type === "button").onClick();
    await mocks.forms[0].onSubmit({ preventDefault: vi.fn() });

    expect(setters[1]).toHaveBeenCalledWith("root");
    expect(setters[2]).toHaveBeenCalledWith("next");
    expect((setters[7] as any).lastValue).toBe(true);
    expect(setters[4]).toHaveBeenCalledWith(true);
    expect(globalThis.fetch).toHaveBeenCalledWith(
      "/api/auth/login",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ username: "admin", password: "secret", passwordConfirm: "" }),
      }),
    );
    expect(win.location.href).toBe("/dashboard");
    expect(setters[4]).toHaveBeenLastCalledWith(false);
  });

  it("submits setup requests and reports server or load errors", async () => {
    installWindow();
    (globalThis as any).fetch = vi.fn(async () => response({ error: "用户名已存在" }, false));
    const { setters } = renderLogin({
      0: { setupRequired: true, authenticated: false },
      1: "admin",
      2: "long-secret",
      3: "long-secret",
      6: "setup-secret",
      7: true,
      8: true,
    });

    expect(mocks.inputs.find((input) => input.placeholder === "密码").type).toBe("text");
    expect(mocks.inputs.find((input) => input.placeholder === "确认密码").type).toBe("text");

    await mocks.forms[0].onSubmit({ preventDefault: vi.fn() });

    expect(globalThis.fetch).toHaveBeenCalledWith(
      "/api/setup/admin",
      expect.objectContaining({
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "X-SubBoost-Setup-Token": "setup-secret",
        },
        body: JSON.stringify({ username: "admin", password: "long-secret", passwordConfirm: "long-secret" }),
      }),
    );
    expect(setters[5]).toHaveBeenCalledWith("用户名已存在");
    expect(setters[4]).toHaveBeenLastCalledWith(false);

    (globalThis as any).fetch = vi.fn(async () => {
      throw new Error("加载失败");
    });
    const load = renderLogin({}, true);
    await flushPromises();
    expect(load.setters[5]).toHaveBeenCalledWith("加载失败");
  });

  it("requires the installer fragment and clears it after successful setup", async () => {
    const missingWindow = installWindow();
    (globalThis as any).fetch = vi.fn();
    const missing = renderLogin({
      0: { setupRequired: true, authenticated: false },
      1: "admin",
      2: "long-secret",
      3: "long-secret",
    });
    expect(missing.html).toContain("安装器输出的初始化链接");
    await mocks.forms[0].onSubmit({ preventDefault: vi.fn() });
    expect(missing.setters[5]).toHaveBeenCalledWith("缺少初始化令牌，请使用安装器输出的初始化链接");
    expect(globalThis.fetch).not.toHaveBeenCalled();

    const win = installWindow("#setup-token=setup-secret");
    (globalThis as any).fetch = vi.fn(async () => response({ success: true }));
    const setup = renderLogin({
      0: { setupRequired: true, authenticated: false },
      1: "admin",
      2: "long-secret",
      3: "long-secret",
      6: "setup-secret",
    });
    await mocks.forms[0].onSubmit({ preventDefault: vi.fn() });

    expect(win.history.replaceState).toHaveBeenCalledWith(null, "", "/login");
    expect(win.location.href).toBe("/dashboard");
    expect(missingWindow.history.replaceState).not.toHaveBeenCalled();
  });
});
