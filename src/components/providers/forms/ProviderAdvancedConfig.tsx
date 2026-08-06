import { useTranslation } from "react-i18next";
import { useState, useEffect } from "react";
import { ChevronDown, ChevronRight, Coins, ListTree } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
export type PricingModelSourceOption = "inherit" | "request" | "response";

interface ProviderPricingConfig {
  enabled: boolean;
  costMultiplier?: string;
  pricingModelSource: PricingModelSourceOption;
}

interface ModelListProxyFormConfig {
  isGlobalSource: boolean;
  modelsUrl: string;
  stripPrefix: string;
}

interface ProviderAdvancedConfigProps {
  pricingConfig: ProviderPricingConfig;
  onPricingConfigChange: (config: ProviderPricingConfig) => void;
  modelListProxy: ModelListProxyFormConfig;
  onModelListProxyChange: (config: ModelListProxyFormConfig) => void;
}

export function ProviderAdvancedConfig({
  pricingConfig,
  onPricingConfigChange,
  modelListProxy,
  onModelListProxyChange,
}: ProviderAdvancedConfigProps) {
  const { t } = useTranslation();
  const [isPricingConfigOpen, setIsPricingConfigOpen] = useState(
    pricingConfig.enabled,
  );
  const [isModelListProxyOpen, setIsModelListProxyOpen] = useState(
    modelListProxy.isGlobalSource,
  );

  useEffect(() => {
    setIsPricingConfigOpen(pricingConfig.enabled);
  }, [pricingConfig.enabled]);

  useEffect(() => {
    setIsModelListProxyOpen(modelListProxy.isGlobalSource);
  }, [modelListProxy.isGlobalSource]);

  return (
    <div className="space-y-4">
      {/* 计费配置 */}
      <div className="rounded-lg border border-border/50 bg-muted/20">
        <button
          type="button"
          className="flex w-full items-center justify-between p-4 hover:bg-muted/30 transition-colors"
          onClick={() => setIsPricingConfigOpen(!isPricingConfigOpen)}
        >
          <div className="flex items-center gap-3">
            <Coins className="h-4 w-4 text-muted-foreground" />
            <span className="font-medium">
              {t("providerAdvanced.pricingConfig", {
                defaultValue: "计费配置",
              })}
            </span>
          </div>
          <div className="flex items-center gap-3">
            <div
              className="flex items-center gap-2"
              onClick={(e) => e.stopPropagation()}
            >
              <Label
                htmlFor="pricing-config-enabled"
                className="text-sm text-muted-foreground"
              >
                {t("providerAdvanced.useCustomPricing", {
                  defaultValue: "使用单独配置",
                })}
              </Label>
              <Switch
                id="pricing-config-enabled"
                checked={pricingConfig.enabled}
                onCheckedChange={(checked) => {
                  onPricingConfigChange({ ...pricingConfig, enabled: checked });
                  if (checked) setIsPricingConfigOpen(true);
                }}
              />
            </div>
            {isPricingConfigOpen ? (
              <ChevronDown className="h-4 w-4 text-muted-foreground" />
            ) : (
              <ChevronRight className="h-4 w-4 text-muted-foreground" />
            )}
          </div>
        </button>
        <div
          className={cn(
            "overflow-hidden transition-all duration-200",
            isPricingConfigOpen
              ? "max-h-[500px] opacity-100"
              : "max-h-0 opacity-0",
          )}
        >
          <div className="border-t border-border/50 p-4 space-y-4">
            <p className="text-sm text-muted-foreground">
              {t("providerAdvanced.pricingConfigDesc", {
                defaultValue:
                  "为此供应商配置单独的计费参数，不启用时使用全局默认配置。",
              })}
            </p>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="cost-multiplier">
                  {t("providerAdvanced.costMultiplier", {
                    defaultValue: "成本倍率",
                  })}
                </Label>
                <Input
                  id="cost-multiplier"
                  type="number"
                  step="0.01"
                  min="0"
                  inputMode="decimal"
                  value={pricingConfig.costMultiplier || ""}
                  onChange={(e) =>
                    onPricingConfigChange({
                      ...pricingConfig,
                      costMultiplier: e.target.value || undefined,
                    })
                  }
                  placeholder={t("providerAdvanced.costMultiplierPlaceholder", {
                    defaultValue: "留空使用全局默认（1）",
                  })}
                  disabled={!pricingConfig.enabled}
                />
                <p className="text-xs text-muted-foreground">
                  {t("providerAdvanced.costMultiplierHint", {
                    defaultValue: "实际成本 = 基础成本 × 倍率，支持小数如 1.5",
                  })}
                </p>
              </div>
              <div className="space-y-2">
                <Label htmlFor="pricing-model-source">
                  {t("providerAdvanced.pricingModelSourceLabel", {
                    defaultValue: "计费模式",
                  })}
                </Label>
                <Select
                  value={pricingConfig.pricingModelSource}
                  onValueChange={(value) =>
                    onPricingConfigChange({
                      ...pricingConfig,
                      pricingModelSource: value as PricingModelSourceOption,
                    })
                  }
                  disabled={!pricingConfig.enabled}
                >
                  <SelectTrigger id="pricing-model-source">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="inherit">
                      {t("providerAdvanced.pricingModelSourceInherit", {
                        defaultValue: "继承全局默认",
                      })}
                    </SelectItem>
                    <SelectItem value="request">
                      {t("providerAdvanced.pricingModelSourceRequest", {
                        defaultValue: "请求模型",
                      })}
                    </SelectItem>
                    <SelectItem value="response">
                      {t("providerAdvanced.pricingModelSourceResponse", {
                        defaultValue: "返回模型",
                      })}
                    </SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  {t("providerAdvanced.pricingModelSourceHint", {
                    defaultValue: "选择按请求模型还是返回模型进行定价匹配",
                  })}
                </p>
              </div>
            </div>
          </div>
        </div>
      </div>

      <div className="rounded-lg border border-border/50 bg-muted/20">
        <button
          type="button"
          className="flex w-full items-center justify-between p-4 hover:bg-muted/30 transition-colors"
          onClick={() => setIsModelListProxyOpen(!isModelListProxyOpen)}
        >
          <div className="flex items-center gap-3">
            <ListTree className="h-4 w-4 text-muted-foreground" />
            <span className="font-medium">
              {t("providerAdvanced.modelListProxy", {
                defaultValue: "全局 Models 代理",
              })}
            </span>
          </div>
          <div className="flex items-center gap-3">
            <div
              className="flex items-center gap-2"
              onClick={(event) => event.stopPropagation()}
            >
              <Label
                htmlFor="model-list-proxy-enabled"
                className="text-sm text-muted-foreground"
              >
                {t("providerAdvanced.useAsGlobalModelsSource", {
                  defaultValue: "作为全局数据源",
                })}
              </Label>
              <Switch
                id="model-list-proxy-enabled"
                checked={modelListProxy.isGlobalSource}
                onCheckedChange={(checked) => {
                  onModelListProxyChange({
                    ...modelListProxy,
                    isGlobalSource: checked,
                  });
                  if (checked) setIsModelListProxyOpen(true);
                }}
              />
            </div>
            {isModelListProxyOpen ? (
              <ChevronDown className="h-4 w-4 text-muted-foreground" />
            ) : (
              <ChevronRight className="h-4 w-4 text-muted-foreground" />
            )}
          </div>
        </button>
        <div
          className={cn(
            "overflow-hidden transition-all duration-200",
            isModelListProxyOpen
              ? "max-h-[500px] opacity-100"
              : "max-h-0 opacity-0",
          )}
        >
          <div className="border-t border-border/50 p-4 space-y-4">
            <p className="text-sm text-muted-foreground">
              {t("providerAdvanced.modelListProxyDesc", {
                defaultValue:
                  "将该供应商的完整 models 响应暴露到本地 /v1/models，并自动转换公开模型名称。全局只能启用一个供应商。",
              })}
            </p>
            <div className="space-y-2">
              <Label htmlFor="model-list-url">
                {t("providerAdvanced.modelsUrl", {
                  defaultValue: "Models API 地址",
                })}
              </Label>
              <Input
                id="model-list-url"
                value={modelListProxy.modelsUrl}
                onChange={(event) =>
                  onModelListProxyChange({
                    ...modelListProxy,
                    modelsUrl: event.target.value,
                  })
                }
                placeholder="https://api.example.com/v1/models"
                disabled={!modelListProxy.isGlobalSource}
              />
              <p className="text-xs text-muted-foreground">
                {t("providerAdvanced.modelsUrlHint", {
                  defaultValue:
                    "可留空，将根据供应商 Base URL 自动推导 /v1/models 或 /models。",
                })}
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="model-list-strip-prefix">
                {t("providerAdvanced.stripModelPrefix", {
                  defaultValue: "删除模型名前缀",
                })}
              </Label>
              <Input
                id="model-list-strip-prefix"
                value={modelListProxy.stripPrefix}
                onChange={(event) =>
                  onModelListProxyChange({
                    ...modelListProxy,
                    stripPrefix: event.target.value,
                  })
                }
                placeholder="claude-"
                disabled={!modelListProxy.isGlobalSource}
              />
              <p className="text-xs text-muted-foreground">
                {t("providerAdvanced.stripModelPrefixHint", {
                  defaultValue:
                    "例如 claude-qd/auto 将显示为 qd/auto；实际请求会自动恢复原始前缀。",
                })}
              </p>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
