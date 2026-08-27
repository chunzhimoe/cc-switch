import { useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, ShieldAlert } from "lucide-react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { FormLabel } from "@/components/ui/form";
import { Switch } from "@/components/ui/switch";

interface AutoClassifierBypassFieldProps {
  value: boolean;
  onChange: (value: boolean) => void;
  classifierRoutingConfigured?: boolean;
}

export function AutoClassifierBypassField({
  value,
  onChange,
  classifierRoutingConfigured = false,
}: AutoClassifierBypassFieldProps) {
  const { t } = useTranslation();
  const [confirmOpen, setConfirmOpen] = useState(false);

  const handleCheckedChange = (checked: boolean) => {
    if (!checked) {
      onChange(false);
      return;
    }
    setConfirmOpen(true);
  };

  return (
    <div className="space-y-3 border-t border-border-default pt-4">
      <div className="space-y-1">
        <FormLabel className="flex items-center gap-2">
          <ShieldAlert className="h-4 w-4 text-destructive" />
          {t("providerForm.skipAutoClassifierTitle", {
            defaultValue: "跳过 Auto 分类器（高风险）",
          })}
        </FormLabel>
        <p className="text-xs text-muted-foreground">
          {t("providerForm.skipAutoClassifierDescription", {
            defaultValue:
              "保存并切换到此供应商时，将 Claude Code live 配置设为 sandbox.enabled=true 与 permissions.defaultMode=bypassPermissions，直连和代理模式都会生效。",
          })}
        </p>
      </div>

      <label className="flex items-center justify-between gap-3 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm">
        <span>
          {t("providerForm.skipAutoClassifierEnabled", {
            defaultValue: "直接跳过权限分类器",
          })}
        </span>
        <Switch checked={value} onCheckedChange={handleCheckedChange} />
      </label>

      {value && (
        <div
          role="alert"
          className="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/10 p-3 text-xs text-destructive"
        >
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
          <p>
            {t("providerForm.skipAutoClassifierRiskHint", {
              defaultValue:
                "该设置会绕过 Auto 模式的逐操作安全分类。保存后必须完全重启 Claude Code；请仅在你信任任务内容并接受 bypassPermissions 风险时启用。",
            })}
          </p>
        </div>
      )}

      {value && classifierRoutingConfigured && (
        <p className="text-xs text-amber-700 dark:text-amber-300" role="status">
          {t("providerForm.skipAutoClassifierRoutingPaused", {
            defaultValue:
              "已保存的分类器分流配置会继续保留，但跳过开关启用期间不会生效；关闭后自动恢复。",
          })}
        </p>
      )}

      <ConfirmDialog
        isOpen={confirmOpen}
        title={t("providerForm.skipAutoClassifierConfirmTitle", {
          defaultValue: "确认跳过 Auto 分类器？",
        })}
        message={t("providerForm.skipAutoClassifierConfirmMessage", {
          defaultValue:
            "这会为该供应商写入以下 Claude Code live 配置：\n\npermissions.defaultMode = bypassPermissions\nsandbox.enabled = true\n\nAuto 分类器将不再逐项判断操作。该变更风险较高，并需要完全重启 Claude Code 后生效。",
        })}
        confirmText={t("providerForm.skipAutoClassifierConfirm", {
          defaultValue: "仍要启用",
        })}
        variant="destructive"
        zIndex="top"
        onConfirm={() => {
          setConfirmOpen(false);
          onChange(true);
        }}
        onCancel={() => setConfirmOpen(false)}
      />
    </div>
  );
}
