import { renderHook, act, waitFor } from "@testing-library/react";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { useDirectorySettings } from "@/hooks/useDirectorySettings";
import type { SettingsFormState } from "@/hooks/useSettingsForm";

const getAppConfigDirOverrideMock = vi.hoisted(() => vi.fn());
const getConfigDirMock = vi.hoisted(() => vi.fn());
const selectConfigDirectoryMock = vi.hoisted(() => vi.fn());
const setAppConfigDirOverrideMock = vi.hoisted(() => vi.fn());
const homeDirMock = vi.hoisted(() => vi.fn<() => Promise<string>>());
const configDirMock = vi.hoisted(() => vi.fn<() => Promise<string>>());
const joinMock = vi.hoisted(() =>
  vi.fn(async (...segments: string[]) => segments.join("/")),
);
const toastErrorMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/api", () => ({
  settingsApi: {
    getAppConfigDirOverride: getAppConfigDirOverrideMock,
    getConfigDir: getConfigDirMock,
    selectConfigDirectory: selectConfigDirectoryMock,
    setAppConfigDirOverride: setAppConfigDirOverrideMock,
  },
}));

vi.mock("@tauri-apps/api/path", () => ({
  homeDir: homeDirMock,
  configDir: configDirMock,
  join: joinMock,
}));

vi.mock("sonner", () => ({
  toast: {
    error: (...args: unknown[]) => toastErrorMock(...args),
  },
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: Record<string, unknown>) =>
      (options?.defaultValue as string) ?? key,
  }),
}));

const createSettings = (
  overrides: Partial<SettingsFormState> = {},
): SettingsFormState => ({
  showInTray: true,
  minimizeToTrayOnClose: true,
  enableClaudePluginIntegration: false,
  claudeConfigDir: "/claude/custom",
  codexConfigDir: "/codex/custom",
  grokConfigDir: "/grok/custom",
  language: "zh",
  ...overrides,
});

describe("useDirectorySettings", () => {
  const onUpdateSettings = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();

    homeDirMock.mockResolvedValue("/home/mock");
    configDirMock.mockResolvedValue("/config/mock");
    joinMock.mockImplementation(async (...segments: string[]) =>
      segments.join("/"),
    );

    getAppConfigDirOverrideMock.mockResolvedValue(null);
    getConfigDirMock.mockImplementation(async (app: string) => {
      if (app === "claude") return "/remote/claude";
      if (app === "codex") return "/remote/codex";
      if (app === "gemini") return "/remote/gemini";
      if (app === "grokbuild") return "/remote/grok";
      if (app === "opencode") return "/remote/opencode";
      if (app === "openclaw") return "/remote/openclaw";
      if (app === "windsurf") return "/remote/windsurf";
      return "/remote/hermes";
    });
    selectConfigDirectoryMock.mockReset();
  });

  it("initializes directories using overrides and remote defaults", async () => {
    getAppConfigDirOverrideMock.mockResolvedValue("  /override/app  ");

    const { result } = renderHook(() =>
      useDirectorySettings({ settings: createSettings(), onUpdateSettings }),
    );

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    expect(result.current.appConfigDir).toBe("/override/app");
    expect(result.current.resolvedDirs).toEqual({
      appConfig: "/override/app",
      claude: "/remote/claude",
      codex: "/remote/codex",
      gemini: "/remote/gemini",
      grokbuild: "/remote/grok",
      opencode: "/remote/opencode",
      openclaw: "/remote/openclaw",
      hermes: "/remote/hermes",
      windsurfUserData: "/remote/windsurf",
      windsurfSkills: "/home/mock/.codeium/windsurf/skills",
      windsurfMcp: "/config/mock/devin",
      windsurfRules: "/home/mock/.codeium/windsurf/memories",
    });
  });

  it("syncs persisted Windsurf directories after settings load", async () => {
    type HookProps = { settings: SettingsFormState | null };
    const { result, rerender } = renderHook(
      ({ settings }: HookProps) =>
        useDirectorySettings({ settings, onUpdateSettings }),
      { initialProps: { settings: null } as HookProps },
    );

    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.resolvedDirs.windsurfUserData).toBe(
      "/remote/windsurf",
    );

    rerender({
      settings: createSettings({
        windsurfUserDataDir: "  /persisted/user-data  ",
        windsurfSkillsDir: "/persisted/skills",
        windsurfMcpDir: "/persisted/mcp",
        windsurfRulesDir: "/persisted/rules",
      }),
    });

    await waitFor(() =>
      expect(result.current.resolvedDirs).toMatchObject({
        windsurfUserData: "/persisted/user-data",
        windsurfSkills: "/persisted/skills",
        windsurfMcp: "/persisted/mcp",
        windsurfRules: "/persisted/rules",
      }),
    );

    rerender({ settings: createSettings() });

    await waitFor(() =>
      expect(result.current.resolvedDirs).toMatchObject({
        windsurfUserData: "/remote/windsurf",
        windsurfSkills: "/home/mock/.codeium/windsurf/skills",
        windsurfMcp: "/config/mock/devin",
        windsurfRules: "/home/mock/.codeium/windsurf/memories",
      }),
    );
  });

  it("updates claude directory when browsing succeeds", async () => {
    selectConfigDirectoryMock.mockResolvedValue("/picked/claude");

    const { result } = renderHook(() =>
      useDirectorySettings({
        settings: createSettings({ claudeConfigDir: undefined }),
        onUpdateSettings,
      }),
    );

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    await act(async () => {
      await result.current.browseDirectory("claude");
    });

    expect(selectConfigDirectoryMock).toHaveBeenCalledWith("/remote/claude");
    expect(onUpdateSettings).toHaveBeenCalledWith({
      claudeConfigDir: "/picked/claude",
    });
    expect(result.current.resolvedDirs.claude).toBe("/picked/claude");
  });

  it("reports error when directory selection fails", async () => {
    selectConfigDirectoryMock.mockResolvedValue(null);

    const { result } = renderHook(() =>
      useDirectorySettings({ settings: createSettings(), onUpdateSettings }),
    );
    await waitFor(() => expect(result.current.isLoading).toBe(false));

    await act(async () => {
      await result.current.browseDirectory("codex");
    });

    expect(result.current.resolvedDirs.codex).toBe("/remote/codex");
    expect(onUpdateSettings).not.toHaveBeenCalledWith({
      codexConfigDir: expect.anything(),
    });
    expect(selectConfigDirectoryMock).toHaveBeenCalled();

    selectConfigDirectoryMock.mockRejectedValue(new Error("dialog failed"));
    toastErrorMock.mockClear();

    await act(async () => {
      await result.current.browseDirectory("codex");
    });

    expect(toastErrorMock).toHaveBeenCalled();
  });

  it("warns when directory selection promise rejects", async () => {
    selectConfigDirectoryMock.mockRejectedValue(new Error("dialog failed"));

    const { result } = renderHook(() =>
      useDirectorySettings({ settings: createSettings(), onUpdateSettings }),
    );
    await waitFor(() => expect(result.current.isLoading).toBe(false));

    await act(async () => {
      await result.current.browseDirectory("codex");
    });

    expect(toastErrorMock).toHaveBeenCalled();
    expect(onUpdateSettings).not.toHaveBeenCalledWith({
      codexConfigDir: expect.anything(),
    });
  });

  it("updates app config directory via browseAppConfigDir", async () => {
    selectConfigDirectoryMock.mockResolvedValue("  /new/app  ");

    const { result } = renderHook(() =>
      useDirectorySettings({
        settings: createSettings(),
        onUpdateSettings,
      }),
    );
    await waitFor(() => expect(result.current.isLoading).toBe(false));

    await act(async () => {
      await result.current.browseAppConfigDir();
    });

    expect(result.current.appConfigDir).toBe("/new/app");
    expect(selectConfigDirectoryMock).toHaveBeenCalledWith(
      "/home/mock/.cc-switch",
    );
  });

  it("resets directories to computed defaults", async () => {
    const { result } = renderHook(() =>
      useDirectorySettings({
        settings: createSettings({
          claudeConfigDir: "/custom/claude",
          codexConfigDir: "/custom/codex",
        }),
        onUpdateSettings,
      }),
    );
    await waitFor(() => expect(result.current.isLoading).toBe(false));

    await act(async () => {
      await result.current.resetDirectory("claude");
      await result.current.resetDirectory("codex");
      await result.current.resetAppConfigDir();
    });

    expect(onUpdateSettings).toHaveBeenCalledWith({
      claudeConfigDir: undefined,
    });
    expect(onUpdateSettings).toHaveBeenCalledWith({
      codexConfigDir: undefined,
    });
    expect(result.current.resolvedDirs.claude).toBe("/home/mock/.claude");
    expect(result.current.resolvedDirs.codex).toBe("/home/mock/.codex");
    expect(result.current.resolvedDirs.appConfig).toBe("/home/mock/.cc-switch");
  });

  it("updates openclaw directory when browsing succeeds", async () => {
    selectConfigDirectoryMock.mockResolvedValue("/picked/openclaw");

    const { result } = renderHook(() =>
      useDirectorySettings({
        settings: createSettings({ openclawConfigDir: undefined }),
        onUpdateSettings,
      }),
    );

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    await act(async () => {
      await result.current.browseDirectory("openclaw");
    });

    expect(selectConfigDirectoryMock).toHaveBeenCalledWith("/remote/openclaw");
    expect(onUpdateSettings).toHaveBeenCalledWith({
      openclawConfigDir: "/picked/openclaw",
    });
    expect(result.current.resolvedDirs.openclaw).toBe("/picked/openclaw");
  });

  it("updates Windsurf MCP directory independently", async () => {
    selectConfigDirectoryMock.mockResolvedValue("/picked/windsurf-mcp");

    const { result } = renderHook(() =>
      useDirectorySettings({
        settings: createSettings({ windsurfMcpDir: undefined }),
        onUpdateSettings,
      }),
    );

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    await act(async () => {
      await result.current.browseDirectory("windsurfMcp");
    });

    expect(selectConfigDirectoryMock).toHaveBeenCalledWith(
      "/config/mock/devin",
    );
    expect(onUpdateSettings).toHaveBeenCalledWith({
      windsurfMcpDir: "/picked/windsurf-mcp",
    });
    expect(result.current.resolvedDirs.windsurfMcp).toBe(
      "/picked/windsurf-mcp",
    );
  });

  it("resetAllDirectories applies provided resolved values", async () => {
    const { result } = renderHook(() =>
      useDirectorySettings({ settings: createSettings(), onUpdateSettings }),
    );
    await waitFor(() => expect(result.current.isLoading).toBe(false));

    act(() => {
      result.current.resetAllDirectories({
        claude: "/server/claude",
        codex: "/server/codex",
        gemini: "/server/gemini",
        grokbuild: "/server/grok",
        opencode: "/server/opencode",
        openclaw: "/server/openclaw",
      });
    });

    expect(result.current.resolvedDirs.claude).toBe("/server/claude");
    expect(result.current.resolvedDirs.codex).toBe("/server/codex");
    expect(result.current.resolvedDirs.gemini).toBe("/server/gemini");
    expect(result.current.resolvedDirs.grokbuild).toBe("/server/grok");
    expect(result.current.resolvedDirs.opencode).toBe("/server/opencode");
    expect(result.current.resolvedDirs.openclaw).toBe("/server/openclaw");
  });
});
