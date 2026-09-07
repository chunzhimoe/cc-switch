import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { toast } from "sonner";

const { switchAccount, settings, idleAction } = vi.hoisted(() => ({
  switchAccount: vi.fn(),
  settings: { windsurfAppPath: "/Applications/Windsurf.app" },
  idleAction: { isPending: false, mutateAsync: vi.fn() },
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: Record<string, unknown>) => {
      const text = String(options?.defaultValue ?? key);
      return text.replace(/\{\{(\w+)\}\}/g, (_, name: string) =>
        String(options?.[name] ?? ""),
      );
    },
  }),
}));

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), warning: vi.fn(), error: vi.fn() },
}));

vi.mock("@/hooks/useWindsurf", () => ({
  useWindsurfAccounts: () => ({
    data: [
      {
        id: "account-b",
        label: "Account B",
        tokenType: "api-key",
        maskedToken: "sk-ws-***",
        tags: [],
        createdAt: 0,
        lastUsed: 0,
      },
    ],
    isLoading: false,
  }),
  useWindsurfStatus: () => ({
    data: { currentAccountId: "account-a", running: true },
  }),
  useWindsurfActions: () => ({
    switchAccount: { isPending: false, mutateAsync: switchAccount },
    deleteAccount: idleAction,
    detectAppPath: idleAction,
    importLocal: idleAction,
    addByToken: idleAction,
    addByPassword: idleAction,
    oauthLoginStart: idleAction,
    oauthLoginComplete: idleAction,
    oauthLoginCancel: idleAction,
    oauthSubmitCallbackUrl: idleAction,
  }),
}));

vi.mock("@/hooks/useSettings", () => ({
  useSettings: () => ({
    settings,
    updateSettings: vi.fn(),
    saveSettings: vi.fn(),
  }),
}));

vi.mock("@/lib/api", () => ({
  settingsApi: { openExternal: vi.fn() },
}));

vi.mock("@/components/ConfirmDialog", () => ({
  ConfirmDialog: ({
    isOpen,
    onConfirm,
  }: {
    isOpen: boolean;
    onConfirm: () => void;
  }) => (isOpen ? <button onClick={onConfirm}>Confirm switch</button> : null),
}));

import WindsurfAccountsPanel from "@/components/windsurf/WindsurfAccountsPanel";

function confirmSwitch() {
  render(<WindsurfAccountsPanel />);
  fireEvent.click(screen.getByRole("button", { name: "切换并重启" }));
  fireEvent.click(screen.getByRole("button", { name: "Confirm switch" }));
}

describe("WindsurfAccountsPanel switch feedback", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    switchAccount.mockReset();
  });

  it("reports success only when the backend confirms restart", async () => {
    switchAccount.mockResolvedValue({
      accountId: "account-b",
      restarted: true,
      processId: 42,
      warning: null,
    });
    confirmSwitch();

    await waitFor(() =>
      expect(toast.success).toHaveBeenCalledWith(
        "会话写入已校验，Windsurf 已启动；请在客户端确认账号",
      ),
    );
    expect(switchAccount).toHaveBeenCalledWith("account-b");
    expect(toast.warning).not.toHaveBeenCalled();
  });

  it("does not report success for restarted false without a warning", async () => {
    switchAccount.mockResolvedValue({
      accountId: "account-b",
      restarted: false,
      processId: null,
    });
    confirmSwitch();

    await waitFor(() =>
      expect(toast.warning).toHaveBeenCalledWith(
        expect.stringContaining("未确认 Windsurf 已启动"),
      ),
    );
    expect(toast.success).not.toHaveBeenCalled();
    expect(toast.error).not.toHaveBeenCalled();
  });

  it.each([false, true])(
    "shows launch diagnostics rather than success when restarted is %s and warning is set",
    async (restarted) => {
      switchAccount.mockResolvedValue({
        accountId: "account-b",
        restarted,
        warning: "No matching Windsurf process",
      });
      confirmSwitch();

      await waitFor(() =>
        expect(toast.warning).toHaveBeenCalledWith(
          "会话写入已校验，但 Windsurf 未能自动启动：No matching Windsurf process",
        ),
      );
      expect(toast.success).not.toHaveBeenCalled();
    },
  );

  it.each([
    "APP_PATH_NOT_FOUND:windsurf",
    "Windsurf credential refresh failed; the client was not changed",
    "Cannot read the target Devin Safe Storage key",
    "Windsurf login-state verification failed after writing",
  ])("does not report success after failure: %s", async (error) => {
    switchAccount.mockRejectedValue(error);
    confirmSwitch();

    await waitFor(() => expect(toast.error).toHaveBeenCalledWith(error));
    expect(toast.success).not.toHaveBeenCalled();
    expect(toast.warning).not.toHaveBeenCalled();
  });
});
