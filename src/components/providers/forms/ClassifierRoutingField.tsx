import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  Plus,
  Star,
  Trash2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { FormLabel } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import type {
  ClassifierModelEntry,
  ClassifierRoutingConfig,
  ClassifierRoutingStrategy,
} from "@/types";

export const DEFAULT_CLASSIFIER_ROUTING: ClassifierRoutingConfig = {
  enabled: false,
  strategy: "priority_list",
  defaultModel: "",
  models: [],
  logHits: false,
};

interface ClassifierRoutingFieldProps {
  value: ClassifierRoutingConfig;
  onChange: (value: ClassifierRoutingConfig) => void;
}

const cloneConfig = (
  value: ClassifierRoutingConfig,
): ClassifierRoutingConfig => ({
  ...value,
  models: value.models.map((model) => ({ ...model })),
});

const parsePrice = (value: string): number | undefined => {
  if (!value.trim()) return undefined;
  const price = Number(value);
  return Number.isFinite(price) && price >= 0 ? price : undefined;
};

export function hasClassifierRoutingValue(
  value: ClassifierRoutingConfig | undefined,
): boolean {
  return Boolean(
    value &&
      (value.enabled ||
        value.defaultModel.trim() ||
        value.models.length > 0 ||
        value.logHits),
  );
}

export function ClassifierRoutingField({
  value,
  onChange,
}: ClassifierRoutingFieldProps) {
  const { t } = useTranslation();
  const [modelId, setModelId] = useState("");
  const [modelNote, setModelNote] = useState("");
  const [inputPrice, setInputPrice] = useState("");
  const [outputPrice, setOutputPrice] = useState("");
  const [addError, setAddError] = useState("");

  const update = (patch: Partial<ClassifierRoutingConfig>) => {
    onChange({ ...cloneConfig(value), ...patch });
  };

  const updateModels = (models: ClassifierModelEntry[]) => {
    update({ models });
  };

  const addModel = () => {
    const id = modelId.trim();
    if (!id) {
      setAddError(
        t("providerForm.classifierRoutingModelRequired", {
          defaultValue: "请输入模型 ID",
        }),
      );
      return;
    }
    if (value.models.some((model) => model.id === id)) {
      setAddError(
        t("providerForm.classifierRoutingModelDuplicate", {
          defaultValue: "该模型 ID 已存在",
        }),
      );
      return;
    }

    const input = parsePrice(inputPrice);
    const output = parsePrice(outputPrice);
    if (
      (inputPrice.trim() && input === undefined) ||
      (outputPrice.trim() && output === undefined)
    ) {
      setAddError(
        t("providerForm.classifierRoutingPriceInvalid", {
          defaultValue: "单价必须是大于等于 0 的数字",
        }),
      );
      return;
    }

    const nextModels = [
      ...value.models,
      {
        id,
        ...(modelNote.trim() ? { note: modelNote.trim() } : {}),
        ...(input !== undefined ? { inputPrice: input } : {}),
        ...(output !== undefined ? { outputPrice: output } : {}),
      },
    ];
    update({
      models: nextModels,
      defaultModel: value.defaultModel.trim() || id,
    });
    setModelId("");
    setModelNote("");
    setInputPrice("");
    setOutputPrice("");
    setAddError("");
  };

  const removeModel = (index: number) => {
    const removed = value.models[index];
    const models = value.models.filter((_, itemIndex) => itemIndex !== index);
    const defaultModel =
      removed?.id === value.defaultModel
        ? (models[0]?.id ?? "")
        : value.defaultModel;
    update({ models, defaultModel });
  };

  const moveModel = (index: number, direction: -1 | 1) => {
    const target = index + direction;
    if (target < 0 || target >= value.models.length) return;
    const models = [...value.models];
    [models[index], models[target]] = [models[target], models[index]];
    updateModels(models);
  };

  const strategyOptions: Array<{
    value: ClassifierRoutingStrategy;
    label: string;
    description: string;
  }> = [
    {
      value: "fixed",
      label: t("providerForm.classifierRoutingFixed", {
        defaultValue: "固定模型",
      }),
      description: t("providerForm.classifierRoutingFixedHint", {
        defaultValue: "始终使用默认模型",
      }),
    },
    {
      value: "priority_list",
      label: t("providerForm.classifierRoutingPriority", {
        defaultValue: "优先列表",
      }),
      description: t("providerForm.classifierRoutingPriorityHint", {
        defaultValue: "按列表从上到下选择模型",
      }),
    },
    {
      value: "cheapest",
      label: t("providerForm.classifierRoutingCheapest", {
        defaultValue: "最便宜优先",
      }),
      description: t("providerForm.classifierRoutingCheapestHint", {
        defaultValue: "按输入和输出单价之和选择",
      }),
    },
  ];

  const hasPickableModel = Boolean(
    value.defaultModel.trim() ||
      value.models.some((model) => model.id.trim().length > 0),
  );

  return (
    <div className="space-y-4 border-t border-border-default pt-4">
      <div className="space-y-1">
        <FormLabel>
          {t("providerForm.classifierRoutingTitle", {
            defaultValue: "Auto 模式分类器分流",
          })}
        </FormLabel>
        <p className="text-xs text-muted-foreground">
          {t("providerForm.classifierRoutingDescription", {
            defaultValue:
              "主模型仍由本供应商的模型映射决定；此处为 Claude Code Auto 模式安全分类器单独选择模型。",
          })}
        </p>
      </div>

      <label className="flex items-center gap-2 text-sm">
        <Checkbox
          checked={value.enabled}
          onCheckedChange={(checked) => update({ enabled: checked === true })}
        />
        {t("providerForm.classifierRoutingEnabled", {
          defaultValue: "启用分类器分流",
        })}
      </label>
      <p className="text-xs text-muted-foreground">
        {t("providerForm.classifierRoutingScopeHint", {
          defaultValue:
            "直连外部 API URL 时通过 Claude Code 的 CLAUDE_CODE_AUTO_MODE_MODEL 生效，无需本地代理；代理模式仍按请求分流。",
        })}
      </p>

      {value.enabled && (
        <div
          role="status"
          className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-xs text-amber-800 dark:text-amber-200"
        >
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
          <p>
            {t("providerForm.classifierRoutingRestartRequired", {
              defaultValue:
                "保存当前供应商后，请完全重启正在运行的 Claude Code；已启动的会话不会重新读取分类器模型环境变量。",
            })}
          </p>
        </div>
      )}

      {value.enabled && !hasPickableModel && (
        <p className="text-xs text-destructive" role="alert">
          {t("providerForm.classifierRoutingNoPickableModel", {
            defaultValue: "请至少添加一个可用的分类器模型后再保存。",
          })}
        </p>
      )}

      <div className="space-y-2">
        <FormLabel>
          {t("providerForm.classifierRoutingStrategy", {
            defaultValue: "分流策略",
          })}
        </FormLabel>
        <div className="grid gap-2 md:grid-cols-3">
          {strategyOptions.map((option) => (
            <label
              key={option.value}
              className="flex cursor-pointer items-start gap-2 rounded-md border border-input p-2 text-sm"
            >
              <input
                type="radio"
                name="classifier-routing-strategy"
                value={option.value}
                checked={value.strategy === option.value}
                onChange={() => update({ strategy: option.value })}
                className="mt-1"
              />
              <span>
                <span className="block font-medium">{option.label}</span>
                <span className="block text-xs text-muted-foreground">
                  {option.description}
                </span>
              </span>
            </label>
          ))}
        </div>
      </div>

      <div className="space-y-2">
        <FormLabel>
          {t("providerForm.classifierRoutingModels", {
            defaultValue: "分类器模型列表",
          })}
        </FormLabel>
        <div className="space-y-2">
          {value.models.map((model, index) => (
            <div
              key={`${model.id}-${index}`}
              className="grid grid-cols-[minmax(0,1fr)_auto] gap-2 rounded-md border border-input p-2 md:grid-cols-[minmax(0,1fr)_auto_auto_auto_auto] md:items-center"
            >
              <div className="min-w-0">
                <div className="flex items-center gap-2 truncate text-sm font-medium">
                  <span className="text-muted-foreground">{index + 1}.</span>
                  <span className="truncate">{model.id}</span>
                  {model.id === value.defaultModel && (
                    <span className="shrink-0 text-xs text-amber-600">
                      <Star className="mr-0.5 inline h-3 w-3 fill-current" />
                      {t("providerForm.classifierRoutingDefault", {
                        defaultValue: "默认",
                      })}
                    </span>
                  )}
                </div>
                {(model.note ||
                  model.inputPrice !== undefined ||
                  model.outputPrice !== undefined) && (
                  <p className="truncate text-xs text-muted-foreground">
                    {[
                      model.note,
                      model.inputPrice !== undefined
                        ? `in ${model.inputPrice}`
                        : undefined,
                      model.outputPrice !== undefined
                        ? `out ${model.outputPrice}`
                        : undefined,
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </p>
                )}
              </div>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => update({ defaultModel: model.id })}
                disabled={model.id === value.defaultModel}
              >
                {t("providerForm.classifierRoutingSetDefault", {
                  defaultValue: "设为默认",
                })}
              </Button>
              <div className="flex justify-end gap-1">
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8"
                  onClick={() => moveModel(index, -1)}
                  disabled={index === 0}
                  aria-label={t("providerForm.classifierRoutingMoveUp", {
                    defaultValue: "上移",
                  })}
                >
                  <ArrowUp className="h-4 w-4" />
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8"
                  onClick={() => moveModel(index, 1)}
                  disabled={index === value.models.length - 1}
                  aria-label={t("providerForm.classifierRoutingMoveDown", {
                    defaultValue: "下移",
                  })}
                >
                  <ArrowDown className="h-4 w-4" />
                </Button>
              </div>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="h-8 w-8 text-destructive"
                onClick={() => removeModel(index)}
                aria-label={t("providerForm.classifierRoutingDelete", {
                  defaultValue: "删除",
                })}
              >
                <Trash2 className="h-4 w-4" />
              </Button>
            </div>
          ))}
          {value.models.length === 0 && (
            <p className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
              {t("providerForm.classifierRoutingNoModels", {
                defaultValue: "尚未添加分类器模型",
              })}
            </p>
          )}
        </div>
      </div>

      <div className="space-y-2 rounded-md border border-dashed p-3">
        <FormLabel>
          {t("providerForm.classifierRoutingAddModel", {
            defaultValue: "+ 添加模型",
          })}
        </FormLabel>
        <div className="grid gap-2 md:grid-cols-2">
          <Input
            value={modelId}
            onChange={(event) => setModelId(event.target.value)}
            placeholder={t("providerForm.classifierRoutingModelIdPlaceholder", {
              defaultValue: "模型 ID，例如 gpt-5.6-sol",
            })}
            autoComplete="off"
          />
          <Input
            value={modelNote}
            onChange={(event) => setModelNote(event.target.value)}
            placeholder={t("providerForm.classifierRoutingNotePlaceholder", {
              defaultValue: "备注（可选）",
            })}
            autoComplete="off"
          />
          <Input
            type="number"
            min="0"
            step="any"
            value={inputPrice}
            onChange={(event) => setInputPrice(event.target.value)}
            placeholder={t(
              "providerForm.classifierRoutingInputPricePlaceholder",
              {
                defaultValue: "输入单价（可选）",
              },
            )}
            aria-label={t("providerForm.classifierRoutingInputPrice", {
              defaultValue: "输入单价",
            })}
          />
          <Input
            type="number"
            min="0"
            step="any"
            value={outputPrice}
            onChange={(event) => setOutputPrice(event.target.value)}
            placeholder={t(
              "providerForm.classifierRoutingOutputPricePlaceholder",
              {
                defaultValue: "输出单价（可选）",
              },
            )}
            aria-label={t("providerForm.classifierRoutingOutputPrice", {
              defaultValue: "输出单价",
            })}
          />
        </div>
        {addError && <p className="text-xs text-destructive">{addError}</p>}
        <Button type="button" variant="outline" size="sm" onClick={addModel}>
          <Plus className="mr-1 h-3.5 w-3.5" />
          {t("providerForm.classifierRoutingAddButton", {
            defaultValue: "添加",
          })}
        </Button>
      </div>

      <label className="flex items-center gap-2 text-sm">
        <Checkbox
          checked={value.logHits}
          onCheckedChange={(checked) => update({ logHits: checked === true })}
        />
        {t("providerForm.classifierRoutingLogHits", {
          defaultValue: "本地代理命中时写调试日志",
        })}
      </label>
    </div>
  );
}
