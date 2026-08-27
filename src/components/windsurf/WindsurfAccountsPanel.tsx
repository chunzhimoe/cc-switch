import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  CheckCircle2,
  Download,
  Globe,
  Loader2,
  Plus,
  RefreshCw,
  Trash2,
} from "lucide-react";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ProviderIcon } from "@/components/ProviderIcon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  useWindsurfAccounts,
  useWindsurfActions,
  useWindsurfStatus,
} from "@/hooks/useWindsurf";
import { settingsApi } from "@/lib/api";
import type {
  WindsurfAccountSummary,
  WindsurfOAuthStartResponse,
} from "@/lib/api/windsurf";

export default function WindsurfAccountsPanel() {
  const { t } = useTranslation();
  const { data: accounts = [], isLoading } = useWindsurfAccounts();
  const { data: status } = useWindsurfStatus();
  const actions = useWindsurfActions();
  const [tokenDialogOpen, setTokenDialogOpen] = useState(false);
  const [passwordDialogOpen, setPasswordDialogOpen] = useState(false);
  const [oauthDialogOpen, setOauthDialogOpen] = useState(false);
  const [token, setToken] = useState("");
  const [label, setLabel] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [oauthSession, setOauthSession] =
    useState<WindsurfOAuthStartResponse | null>(null);
  const [oauthWaiting, setOauthWaiting] = useState(false);
  const [oauthError, setOauthError] = useState<string | null>(null);
  const [manualCallbackUrl, setManualCallbackUrl] = useState("");
  const [pendingSwitch, setPendingSwitch] =
    useState<WindsurfAccountSummary | null>(null);
  const [pendingDelete, setPendingDelete] =
    useState<WindsurfAccountSummary | null>(null);
  const oauthLoginIdRef = useRef<string | null>(null);
  const oauthFlowRef = useRef(0);
  const oauthCancelledRef = useRef(false);

  const resetOauthState = () => {
    setOauthSession(null);
    setOauthWaiting(false);
    setOauthError(null);
    setManualCallbackUrl("");
    oauthLoginIdRef.current = null;
  };

  const cancelOauthSession = async () => {
    oauthCancelledRef.current = true;
    oauthFlowRef.current += 1;
    const loginId = oauthLoginIdRef.current;
    oauthLoginIdRef.current = null;
    if (loginId) {
      try {
        await actions.oauthLoginCancel.mutateAsync(loginId);
      } catch {
        // Best-effort cancel; UI already closing.
      }
    }
  };

  useEffect(() => {
    return () => {
      oauthCancelledRef.current = true;
      oauthFlowRef.current += 1;
      if (oauthLoginIdRef.current) {
        void actions.oauthLoginCancel.mutateAsync(oauthLoginIdRef.current);
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const startOauthFlow = async () => {
    const flowId = oauthFlowRef.current + 1;
    oauthFlowRef.current = flowId;
    oauthCancelledRef.current = false;
    setOauthError(null);
    setOauthWaiting(false);
    setManualCallbackUrl("");
    try {
      const session = await actions.oauthLoginStart.mutateAsync();
      if (oauthFlowRef.current !== flowId || oauthCancelledRef.current) {
        await actions.oauthLoginCancel.mutateAsync(session.loginId);
        return;
      }
      oauthLoginIdRef.current = session.loginId;
      setOauthSession(session);
      try {
        await settingsApi.openExternal(session.verificationUri);
      } catch (error) {
        console.debug("[WindsurfOAuth] Failed to open browser:", error);
      }
      if (oauthFlowRef.current !== flowId || oauthCancelledRef.current) {
        return;
      }
      setOauthWaiting(true);
      try {
        await actions.oauthLoginComplete.mutateAsync(session.loginId);
        if (oauthFlowRef.current !== flowId || oauthCancelledRef.current) {
          return;
        }
        resetOauthState();
        setOauthDialogOpen(false);
        toast.success(
          t("windsurf.notifications.added", {
            defaultValue: "Windsurf 账号已添加",
          }),
        );
      } catch (error) {
        if (oauthFlowRef.current !== flowId || oauthCancelledRef.current) {
          return;
        }
        setOauthWaiting(false);
        setOauthError(String(error));
      }
    } catch (error) {
      if (oauthFlowRef.current !== flowId || oauthCancelledRef.current) {
        return;
      }
      setOauthError(String(error));
    }
  };

  const handleOpenOauthDialog = () => {
    setOauthDialogOpen(true);
    void startOauthFlow();
  };

  const handleCloseOauthDialog = async (open: boolean) => {
    if (open) {
      setOauthDialogOpen(true);
      return;
    }
    await cancelOauthSession();
    resetOauthState();
    setOauthDialogOpen(false);
  };

  const handleSubmitCallbackUrl = async () => {
    if (!oauthSession || !manualCallbackUrl.trim()) return;
    setOauthError(null);
    try {
      await actions.oauthSubmitCallbackUrl.mutateAsync({
        loginId: oauthSession.loginId,
        callbackUrl: manualCallbackUrl.trim(),
      });
      toast.success(
        t("windsurf.oauth.callbackAccepted", {
          defaultValue: "已接收回调链接，正在完成登录…",
        }),
      );
    } catch (error) {
      setOauthError(String(error));
    }
  };

  const handleImportLocal = async () => {
    try {
      const account = await actions.importLocal.mutateAsync();
      toast.success(
        t("windsurf.notifications.imported", {
          defaultValue: "已导入 Windsurf 账号：{{label}}",
          label: account.label,
        }),
      );
    } catch (error) {
      toast.error(String(error));
    }
  };

  const handleAddToken = async () => {
    if (!token.trim()) return;
    try {
      await actions.addByToken.mutateAsync({
        token: token.trim(),
        label: label.trim() || undefined,
      });
      setToken("");
      setLabel("");
      setTokenDialogOpen(false);
      toast.success(
        t("windsurf.notifications.added", {
          defaultValue: "Windsurf 账号已添加",
        }),
      );
    } catch (error) {
      toast.error(String(error));
    }
  };

  const handleAddPassword = async () => {
    if (!email.trim() || !password) return;
    try {
      await actions.addByPassword.mutateAsync({
        email: email.trim(),
        password,
        label: label.trim() || undefined,
      });
      setEmail("");
      setPassword("");
      setLabel("");
      setPasswordDialogOpen(false);
      toast.success(
        t("windsurf.notifications.added", {
          defaultValue: "Windsurf 账号已添加",
        }),
      );
    } catch (error) {
      toast.error(String(error));
    }
  };

  const handleSwitch = async (account: WindsurfAccountSummary) => {
    try {
      const result = await actions.switchAccount.mutateAsync(account.id);
      if (result.warning) {
        toast.warning(
          t("windsurf.notifications.switchedWithWarning", {
            defaultValue: "账号已切换，但 Windsurf 未能自动启动：{{warning}}",
            warning: result.warning,
          }),
        );
      } else {
        toast.success(
          t("windsurf.notifications.switched", {
            defaultValue: "已切换并重启 Windsurf",
          }),
        );
      }
    } catch (error) {
      toast.error(String(error));
    } finally {
      setPendingSwitch(null);
    }
  };

  const handleDelete = async (account: WindsurfAccountSummary) => {
    try {
      await actions.deleteAccount.mutateAsync(account.id);
      toast.success(
        t("windsurf.notifications.deleted", {
          defaultValue: "Windsurf 账号已删除",
        }),
      );
    } catch (error) {
      toast.error(String(error));
    } finally {
      setPendingDelete(null);
    }
  };

  return (
    <div className="px-6 py-5 space-y-4">
      <section className="rounded-xl border border-border-default bg-card p-4">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0 space-y-1">
            <div className="flex items-center gap-2">
              <ProviderIcon icon="windsurf" name="Windsurf" size={22} />
              <h2 className="text-base font-semibold">
                {t("windsurf.title", { defaultValue: "Windsurf 账号" })}
              </h2>
              <Badge variant={status?.running ? "default" : "secondary"}>
                {status?.running
                  ? t("windsurf.status.running", { defaultValue: "运行中" })
                  : t("windsurf.status.stopped", { defaultValue: "未运行" })}
              </Badge>
            </div>
            <p
              className="truncate text-xs text-muted-foreground"
              title={status?.appPath ?? undefined}
            >
              {status?.appPath ||
                t("windsurf.status.pathMissing", {
                  defaultValue: "尚未检测到 Windsurf/Devin 可执行文件",
                })}
            </p>
            {status?.userDataDir && (
              <p
                className="truncate text-xs text-muted-foreground"
                title={status.userDataDir}
              >
                {status.userDataDir}
              </p>
            )}
          </div>
          <div className="flex shrink-0 flex-wrap justify-end gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={actions.detectAppPath.isPending}
              onClick={async () => {
                try {
                  const path = await actions.detectAppPath.mutateAsync();
                  path
                    ? toast.success(
                        t("windsurf.notifications.pathDetected", {
                          defaultValue: "已检测到 Windsurf：{{path}}",
                          path,
                        }),
                      )
                    : toast.warning(
                        t("windsurf.status.pathMissing", {
                          defaultValue: "尚未检测到 Windsurf/Devin 可执行文件",
                        }),
                      );
                } catch (error) {
                  toast.error(String(error));
                }
              }}
            >
              <RefreshCw className="mr-2 h-4 w-4" />
              {t("windsurf.actions.detect", { defaultValue: "重新检测" })}
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={actions.importLocal.isPending}
              onClick={() => void handleImportLocal()}
            >
              <Download className="mr-2 h-4 w-4" />
              {t("windsurf.actions.importLocal", {
                defaultValue: "导入本机账号",
              })}
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                setLabel("");
                setPasswordDialogOpen(true);
              }}
            >
              <Plus className="mr-2 h-4 w-4" />
              {t("windsurf.actions.addPassword", {
                defaultValue: "邮箱密码登录",
              })}
            </Button>
            <Button variant="outline" size="sm" onClick={handleOpenOauthDialog}>
              <Globe className="mr-2 h-4 w-4" />
              {t("windsurf.actions.addOauth", {
                defaultValue: "OAuth 授权",
              })}
            </Button>
            <Button
              size="sm"
              onClick={() => {
                setLabel("");
                setTokenDialogOpen(true);
              }}
            >
              <Plus className="mr-2 h-4 w-4" />
              {t("windsurf.actions.addToken", { defaultValue: "添加 Token" })}
            </Button>
          </div>
        </div>
      </section>

      <section className="space-y-2">
        {isLoading ? (
          <div className="flex justify-center py-12 text-muted-foreground">
            <Loader2 className="h-5 w-5 animate-spin" />
          </div>
        ) : accounts.length === 0 ? (
          <div className="rounded-xl border border-dashed border-border-default p-10 text-center text-sm text-muted-foreground">
            {t("windsurf.empty", {
              defaultValue:
                "还没有 Windsurf 账号。可导入本机登录态、邮箱密码登录、OAuth 授权或添加 Token。",
            })}
          </div>
        ) : (
          accounts.map((account) => {
            const current = status?.currentAccountId === account.id;
            const pending =
              actions.switchAccount.isPending ||
              actions.deleteAccount.isPending;
            return (
              <article
                key={account.id}
                className="flex items-center justify-between gap-4 rounded-xl border border-border-default bg-card px-4 py-3"
              >
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="truncate font-medium">
                      {account.label}
                    </span>
                    {current && (
                      <Badge className="gap-1" variant="secondary">
                        <CheckCircle2 className="h-3 w-3" />
                        {t("windsurf.current", { defaultValue: "当前" })}
                      </Badge>
                    )}
                    <Badge variant="outline">{account.tokenType}</Badge>
                  </div>
                  <div className="mt-1 flex flex-wrap gap-x-3 text-xs text-muted-foreground">
                    {account.email && <span>{account.email}</span>}
                    <span className="font-mono">{account.maskedToken}</span>
                  </div>
                </div>
                <div className="flex shrink-0 gap-2">
                  <Button
                    size="sm"
                    variant={current ? "secondary" : "default"}
                    disabled={current || pending}
                    onClick={() => setPendingSwitch(account)}
                  >
                    {t("windsurf.actions.switch", {
                      defaultValue: "切换并重启",
                    })}
                  </Button>
                  <Button
                    size="icon"
                    variant="ghost"
                    disabled={pending}
                    onClick={() => setPendingDelete(account)}
                    aria-label={t("common.delete")}
                  >
                    <Trash2 className="h-4 w-4 text-destructive" />
                  </Button>
                </div>
              </article>
            );
          })
        )}
      </section>

      <Dialog open={tokenDialogOpen} onOpenChange={setTokenDialogOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>
              {t("windsurf.addToken.title", {
                defaultValue: "添加 Windsurf Token",
              })}
            </DialogTitle>
            <DialogDescription>
              {t("windsurf.addToken.description", {
                defaultValue:
                  "支持 sk-ws-*、cog_*、auth1_* 或 devin-session-token$*。Token 只保存在本机账号文件中。",
              })}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3 px-6 py-5">
            <Input
              value={label}
              onChange={(event) => setLabel(event.target.value)}
              placeholder={t("windsurf.addToken.label", {
                defaultValue: "账号名称（可选）",
              })}
            />
            <Input
              type="password"
              value={token}
              onChange={(event) => setToken(event.target.value)}
              placeholder={t("windsurf.addToken.token", {
                defaultValue: "Token",
              })}
              autoComplete="off"
            />
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setTokenDialogOpen(false)}>
              {t("common.cancel")}
            </Button>
            <Button
              disabled={!token.trim() || actions.addByToken.isPending}
              onClick={() => void handleAddToken()}
            >
              {actions.addByToken.isPending && (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              )}
              {t("common.add")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={passwordDialogOpen} onOpenChange={setPasswordDialogOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>
              {t("windsurf.addPassword.title", {
                defaultValue: "邮箱密码登录 Windsurf",
              })}
            </DialogTitle>
            <DialogDescription>
              {t("windsurf.addPassword.description", {
                defaultValue:
                  "通过 Devin Auth1 邮箱密码登录，换取本机可用的 IDE session。账号信息只保存在本机。",
              })}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3 px-6 py-5">
            <Input
              value={label}
              onChange={(event) => setLabel(event.target.value)}
              placeholder={t("windsurf.addPassword.label", {
                defaultValue: "账号名称（可选）",
              })}
            />
            <Input
              type="email"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
              placeholder={t("windsurf.addPassword.email", {
                defaultValue: "邮箱",
              })}
              autoComplete="username"
            />
            <Input
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder={t("windsurf.addPassword.password", {
                defaultValue: "密码",
              })}
              autoComplete="current-password"
            />
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setPasswordDialogOpen(false)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              disabled={
                !email.trim() || !password || actions.addByPassword.isPending
              }
              onClick={() => void handleAddPassword()}
            >
              {actions.addByPassword.isPending && (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              )}
              {t("common.add")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={oauthDialogOpen} onOpenChange={handleCloseOauthDialog}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>
              {t("windsurf.oauth.title", {
                defaultValue: "OAuth 授权登录 Windsurf",
              })}
            </DialogTitle>
            <DialogDescription>
              {t("windsurf.oauth.description", {
                defaultValue:
                  "在浏览器完成 Windsurf 授权。若回调未能自动返回，可粘贴完整回调 URL。",
              })}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3 px-6 py-5">
            {!oauthSession && !oauthError ? (
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" />
                {t("windsurf.oauth.preparing", {
                  defaultValue: "正在准备授权信息…",
                })}
              </div>
            ) : (
              <>
                {oauthSession && (
                  <>
                    <div className="space-y-1">
                      <label className="text-xs text-muted-foreground">
                        {t("windsurf.oauth.linkLabel", {
                          defaultValue: "授权链接",
                        })}
                      </label>
                      <Input
                        value={oauthSession.verificationUri}
                        readOnly
                        className="font-mono text-xs"
                      />
                    </div>
                    {oauthSession.callbackUrl && (
                      <p className="text-xs text-muted-foreground">
                        {t("windsurf.oauth.callbackHint", {
                          defaultValue: "本地回调地址：{{url}}",
                          url: oauthSession.callbackUrl,
                        })}
                      </p>
                    )}
                    <Button
                      className="w-full"
                      onClick={() => {
                        void settingsApi
                          .openExternal(oauthSession.verificationUri)
                          .catch((error) => {
                            toast.error(String(error));
                          });
                      }}
                    >
                      <Globe className="mr-2 h-4 w-4" />
                      {t("windsurf.oauth.openBrowser", {
                        defaultValue: "在浏览器中打开",
                      })}
                    </Button>
                    <div className="space-y-1">
                      <label className="text-xs text-muted-foreground">
                        {t("windsurf.oauth.manualCallbackLabel", {
                          defaultValue: "手动粘贴回调 URL",
                        })}
                      </label>
                      <div className="flex gap-2">
                        <Input
                          value={manualCallbackUrl}
                          onChange={(event) =>
                            setManualCallbackUrl(event.target.value)
                          }
                          placeholder={t(
                            "windsurf.oauth.manualCallbackPlaceholder",
                            {
                              defaultValue:
                                "http://127.0.0.1:端口/windsurf-auth-callback?access_token=...&state=...",
                            },
                          )}
                          className="font-mono text-xs"
                        />
                        <Button
                          variant="outline"
                          disabled={
                            !manualCallbackUrl.trim() ||
                            actions.oauthSubmitCallbackUrl.isPending
                          }
                          onClick={() => void handleSubmitCallbackUrl()}
                        >
                          {actions.oauthSubmitCallbackUrl.isPending ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            t("windsurf.oauth.submitCallback", {
                              defaultValue: "提交",
                            })
                          )}
                        </Button>
                      </div>
                    </div>
                  </>
                )}
                {oauthWaiting && !oauthError && (
                  <div className="flex items-center gap-2 text-sm text-muted-foreground">
                    <Loader2 className="h-4 w-4 animate-spin" />
                    {t("windsurf.oauth.waiting", {
                      defaultValue: "等待授权完成…",
                    })}
                  </div>
                )}
                {oauthError && (
                  <div className="rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive">
                    {oauthError}
                  </div>
                )}
              </>
            )}
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => void handleCloseOauthDialog(false)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="secondary"
              disabled={actions.oauthLoginStart.isPending}
              onClick={() => {
                void (async () => {
                  await cancelOauthSession();
                  resetOauthState();
                  await startOauthFlow();
                })();
              }}
            >
              {actions.oauthLoginStart.isPending && (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              )}
              {t("windsurf.oauth.retry", {
                defaultValue: "重新授权",
              })}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        isOpen={Boolean(pendingSwitch)}
        title={t("windsurf.switchConfirm.title", {
          defaultValue: "切换 Windsurf 账号",
        })}
        message={t("windsurf.switchConfirm.message", {
          defaultValue:
            "将关闭 Windsurf/Devin、写入新账号登录态并重新启动。请先保存编辑器中的工作。",
        })}
        confirmText={t("windsurf.actions.switch", {
          defaultValue: "切换并重启",
        })}
        variant="info"
        onCancel={() => setPendingSwitch(null)}
        onConfirm={() => {
          if (pendingSwitch) void handleSwitch(pendingSwitch);
        }}
      />

      <ConfirmDialog
        isOpen={Boolean(pendingDelete)}
        title={t("windsurf.deleteConfirm.title", {
          defaultValue: "删除 Windsurf 账号",
        })}
        message={t("windsurf.deleteConfirm.message", {
          defaultValue:
            "将从 cc-switch 删除此账号，不会修改当前 Windsurf 登录态。",
        })}
        confirmText={t("common.delete")}
        onCancel={() => setPendingDelete(null)}
        onConfirm={() => {
          if (pendingDelete) void handleDelete(pendingDelete);
        }}
      />
    </div>
  );
}
