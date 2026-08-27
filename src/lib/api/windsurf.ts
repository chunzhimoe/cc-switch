import { invoke } from "@tauri-apps/api/core";

export interface WindsurfAccountSummary {
  id: string;
  label: string;
  email?: string | null;
  tokenType: string;
  maskedToken: string;
  tags: string[];
  createdAt: number;
  lastUsed: number;
}

export interface WindsurfSwitchResult {
  accountId: string;
  restarted: boolean;
  processId?: number | null;
  warning?: string | null;
}

export interface WindsurfStatus {
  currentAccountId?: string | null;
  running: boolean;
  appPath?: string | null;
  userDataDir: string;
  stateDbPath: string;
  mcpConfigPath?: string | null;
  rulesPath: string;
}

export interface WindsurfOAuthStartResponse {
  loginId: string;
  verificationUri: string;
  verificationUriComplete?: string | null;
  expiresIn: number;
  intervalSeconds: number;
  callbackUrl?: string | null;
}

export const windsurfApi = {
  async listAccounts(): Promise<WindsurfAccountSummary[]> {
    return await invoke("list_windsurf_accounts");
  },

  async importFromLocal(): Promise<WindsurfAccountSummary> {
    return await invoke("import_windsurf_from_local");
  },

  async addByToken(
    token: string,
    label?: string,
  ): Promise<WindsurfAccountSummary> {
    return await invoke("add_windsurf_account_with_token", {
      token,
      label: label ?? null,
    });
  },

  async addByPassword(
    email: string,
    password: string,
    label?: string,
  ): Promise<WindsurfAccountSummary> {
    return await invoke("add_windsurf_account_with_password", {
      email,
      password,
      label: label ?? null,
    });
  },

  async oauthLoginStart(): Promise<WindsurfOAuthStartResponse> {
    return await invoke("windsurf_oauth_login_start");
  },

  async oauthLoginComplete(loginId: string): Promise<WindsurfAccountSummary> {
    return await invoke("windsurf_oauth_login_complete", { loginId });
  },

  async oauthLoginCancel(loginId?: string | null): Promise<void> {
    await invoke("windsurf_oauth_login_cancel", {
      loginId: loginId ?? null,
    });
  },

  async oauthSubmitCallbackUrl(
    loginId: string,
    callbackUrl: string,
  ): Promise<void> {
    await invoke("windsurf_oauth_submit_callback_url", {
      loginId,
      callbackUrl,
    });
  },

  async deleteAccount(accountId: string): Promise<boolean> {
    return await invoke("delete_windsurf_account", { accountId });
  },

  async switchAccount(accountId: string): Promise<WindsurfSwitchResult> {
    return await invoke("switch_windsurf_account", { accountId });
  },

  async detectAppPath(force = false): Promise<string | null> {
    return await invoke("detect_windsurf_app_path", { force });
  },

  async setAppPath(path: string | null): Promise<void> {
    await invoke("set_windsurf_app_path", { path });
  },

  async setUserDataDir(path: string | null): Promise<void> {
    await invoke("set_windsurf_user_data_dir", { path });
  },

  async getStatus(): Promise<WindsurfStatus> {
    return await invoke("get_windsurf_status");
  },
};
