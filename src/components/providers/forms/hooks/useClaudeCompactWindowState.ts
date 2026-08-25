import { useCallback, useEffect, useRef, useState } from "react";

interface UseClaudeCompactWindowStateProps {
  settingsConfig: string;
  onConfigChange: (config: string) => void;
}

export const DEFAULT_CLAUDE_CONTEXT_WINDOW = "400000";

export function isPositiveIntegerString(value: string): boolean {
  if (!/^[1-9]\d*$/.test(value)) return false;
  return Number.isSafeInteger(Number(value));
}

function parseContextWindows(settingsConfig: string) {
  try {
    const config = settingsConfig ? JSON.parse(settingsConfig) : {};
    const maxContextTokens = config?.env?.CLAUDE_CODE_MAX_CONTEXT_TOKENS;
    const envCompactWindow = config?.env?.CLAUDE_CODE_AUTO_COMPACT_WINDOW;
    const topLevelCompactWindow = config?.autoCompactWindow;

    return {
      claudeCodeMaxContextTokens:
        (typeof maxContextTokens === "string" ||
          typeof maxContextTokens === "number") &&
        isPositiveIntegerString(String(maxContextTokens))
          ? String(maxContextTokens)
          : DEFAULT_CLAUDE_CONTEXT_WINDOW,
      claudeCodeAutoCompactWindow:
        (typeof envCompactWindow === "string" ||
          typeof envCompactWindow === "number") &&
        isPositiveIntegerString(String(envCompactWindow))
          ? String(envCompactWindow)
          : DEFAULT_CLAUDE_CONTEXT_WINDOW,
      autoCompactWindow:
        (typeof topLevelCompactWindow === "number" ||
          typeof topLevelCompactWindow === "string") &&
        isPositiveIntegerString(String(topLevelCompactWindow))
          ? String(topLevelCompactWindow)
          : DEFAULT_CLAUDE_CONTEXT_WINDOW,
    };
  } catch {
    return {
      claudeCodeMaxContextTokens: DEFAULT_CLAUDE_CONTEXT_WINDOW,
      claudeCodeAutoCompactWindow: DEFAULT_CLAUDE_CONTEXT_WINDOW,
      autoCompactWindow: DEFAULT_CLAUDE_CONTEXT_WINDOW,
    };
  }
}

export function applyClaudeContextWindowConfig(
  settingsConfig: string,
  claudeCodeMaxContextTokens: string,
  claudeCodeAutoCompactWindow: string,
  autoCompactWindow: string,
): string {
  for (const [field, value] of [
    ["CLAUDE_CODE_MAX_CONTEXT_TOKENS", claudeCodeMaxContextTokens],
    ["CLAUDE_CODE_AUTO_COMPACT_WINDOW", claudeCodeAutoCompactWindow],
    ["autoCompactWindow", autoCompactWindow],
  ] as const) {
    if (!isPositiveIntegerString(value)) {
      throw new Error(`${field} must be a positive integer`);
    }
  }

  const config = settingsConfig ? JSON.parse(settingsConfig) : {};
  if (
    !config.env ||
    typeof config.env !== "object" ||
    Array.isArray(config.env)
  ) {
    config.env = {};
  }
  config.env.CLAUDE_CODE_MAX_CONTEXT_TOKENS = claudeCodeMaxContextTokens;
  config.env.CLAUDE_CODE_AUTO_COMPACT_WINDOW = claudeCodeAutoCompactWindow;
  config.autoCompactWindow = Number(autoCompactWindow);

  return JSON.stringify(config, null, 2);
}

export function useClaudeCompactWindowState({
  settingsConfig,
  onConfigChange,
}: UseClaudeCompactWindowStateProps) {
  const initial = useState(() => parseContextWindows(settingsConfig))[0];
  const [claudeCodeMaxContextTokens, setClaudeCodeMaxContextTokens] = useState(
    initial.claudeCodeMaxContextTokens,
  );
  const [claudeCodeAutoCompactWindow, setClaudeCodeAutoCompactWindow] =
    useState(initial.claudeCodeAutoCompactWindow);
  const [autoCompactWindow, setAutoCompactWindow] = useState(
    initial.autoCompactWindow,
  );

  const isUserEditingRef = useRef(false);
  const lastConfigRef = useRef(settingsConfig);
  const latestConfigRef = useRef(settingsConfig);

  latestConfigRef.current = settingsConfig;

  useEffect(() => {
    if (lastConfigRef.current === settingsConfig) return;
    if (isUserEditingRef.current) {
      isUserEditingRef.current = false;
      lastConfigRef.current = settingsConfig;
      return;
    }

    lastConfigRef.current = settingsConfig;
    const parsed = parseContextWindows(settingsConfig);
    setClaudeCodeMaxContextTokens(parsed.claudeCodeMaxContextTokens);
    setClaudeCodeAutoCompactWindow(parsed.claudeCodeAutoCompactWindow);
    setAutoCompactWindow(parsed.autoCompactWindow);
  }, [settingsConfig]);

  const updateConfig = useCallback(
    (
      field: "maxContextTokens" | "envCompactWindow" | "topLevelCompactWindow",
      value: string,
    ) => {
      isUserEditingRef.current = true;

      try {
        const config = latestConfigRef.current
          ? JSON.parse(latestConfigRef.current)
          : {};

        if (field !== "topLevelCompactWindow") {
          if (
            !config.env ||
            typeof config.env !== "object" ||
            Array.isArray(config.env)
          ) {
            config.env = {};
          }
          const envField =
            field === "maxContextTokens"
              ? "CLAUDE_CODE_MAX_CONTEXT_TOKENS"
              : "CLAUDE_CODE_AUTO_COMPACT_WINDOW";
          if (value) {
            config.env[envField] = value;
          } else {
            delete config.env[envField];
          }
        } else if (value && isPositiveIntegerString(value)) {
          config.autoCompactWindow = Number(value);
        } else {
          delete config.autoCompactWindow;
        }

        const updatedConfig = JSON.stringify(config, null, 2);
        latestConfigRef.current = updatedConfig;
        onConfigChange(updatedConfig);
      } catch (error) {
        console.error("Failed to update Claude context window config:", error);
      }
    },
    [onConfigChange],
  );

  const handleClaudeCodeMaxContextTokensChange = useCallback(
    (value: string) => {
      setClaudeCodeMaxContextTokens(value);
      updateConfig("maxContextTokens", value);
    },
    [updateConfig],
  );

  const handleClaudeCodeAutoCompactWindowChange = useCallback(
    (value: string) => {
      setClaudeCodeAutoCompactWindow(value);
      updateConfig("envCompactWindow", value);
    },
    [updateConfig],
  );

  const handleAutoCompactWindowChange = useCallback(
    (value: string) => {
      setAutoCompactWindow(value);
      updateConfig("topLevelCompactWindow", value);
    },
    [updateConfig],
  );

  return {
    claudeCodeMaxContextTokens,
    claudeCodeAutoCompactWindow,
    autoCompactWindow,
    handleClaudeCodeMaxContextTokensChange,
    handleClaudeCodeAutoCompactWindowChange,
    handleAutoCompactWindowChange,
  };
}
