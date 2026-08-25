import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import {
  applyClaudeContextWindowConfig,
  DEFAULT_CLAUDE_CONTEXT_WINDOW,
  isPositiveIntegerString,
  useClaudeCompactWindowState,
} from "@/components/providers/forms/hooks/useClaudeCompactWindowState";

describe("useClaudeCompactWindowState", () => {
  it("uses 400000 for all context-window fields when they are missing", () => {
    const { result } = renderHook(() =>
      useClaudeCompactWindowState({
        settingsConfig: JSON.stringify({ env: {} }),
        onConfigChange: vi.fn(),
      }),
    );

    expect(result.current.claudeCodeMaxContextTokens).toBe(
      DEFAULT_CLAUDE_CONTEXT_WINDOW,
    );
    expect(result.current.claudeCodeAutoCompactWindow).toBe(
      DEFAULT_CLAUDE_CONTEXT_WINDOW,
    );
    expect(result.current.autoCompactWindow).toBe(
      DEFAULT_CLAUDE_CONTEXT_WINDOW,
    );
  });

  it("hydrates the three values independently", () => {
    const { result } = renderHook(() =>
      useClaudeCompactWindowState({
        settingsConfig: JSON.stringify({
          env: {
            CLAUDE_CODE_MAX_CONTEXT_TOKENS: "390000",
            CLAUDE_CODE_AUTO_COMPACT_WINDOW: "380000",
          },
          autoCompactWindow: 370000,
        }),
        onConfigChange: vi.fn(),
      }),
    );

    expect(result.current.claudeCodeMaxContextTokens).toBe("390000");
    expect(result.current.claudeCodeAutoCompactWindow).toBe("380000");
    expect(result.current.autoCompactWindow).toBe("370000");
  });

  it("writes env values as strings and the top-level value as a number", () => {
    let latestConfig = JSON.stringify({
      env: {
        ANTHROPIC_MODEL: "mapped-model",
        CLAUDE_CODE_MAX_CONTEXT_TOKENS: "390000",
        CLAUDE_CODE_AUTO_COMPACT_WINDOW: "380000",
      },
      autoCompactWindow: 370000,
      theme: "dark",
    });
    const onConfigChange = vi.fn((config: string) => {
      latestConfig = config;
    });

    const { result } = renderHook(() =>
      useClaudeCompactWindowState({
        settingsConfig: latestConfig,
        onConfigChange,
      }),
    );

    act(() => {
      result.current.handleClaudeCodeMaxContextTokensChange("410000");
    });
    act(() => {
      result.current.handleClaudeCodeAutoCompactWindowChange("420000");
    });
    act(() => {
      result.current.handleAutoCompactWindowChange("430000");
    });

    const parsed = JSON.parse(latestConfig);
    expect(parsed.env.CLAUDE_CODE_MAX_CONTEXT_TOKENS).toBe("410000");
    expect(parsed.env.CLAUDE_CODE_AUTO_COMPACT_WINDOW).toBe("420000");
    expect(parsed.autoCompactWindow).toBe(430000);
    expect(parsed.env.ANTHROPIC_MODEL).toBe("mapped-model");
    expect(parsed.theme).toBe("dark");
  });
});

describe("applyClaudeContextWindowConfig", () => {
  it("adds all 400000 defaults while preserving unrelated settings", () => {
    const updated = applyClaudeContextWindowConfig(
      JSON.stringify({
        env: { ANTHROPIC_MODEL: "mapped-model" },
        theme: "dark",
      }),
      "400000",
      "400000",
      "400000",
    );
    const parsed = JSON.parse(updated);

    expect(parsed.env.CLAUDE_CODE_MAX_CONTEXT_TOKENS).toBe("400000");
    expect(parsed.env.CLAUDE_CODE_AUTO_COMPACT_WINDOW).toBe("400000");
    expect(parsed.autoCompactWindow).toBe(400000);
    expect(parsed.env.ANTHROPIC_MODEL).toBe("mapped-model");
    expect(parsed.theme).toBe("dark");
  });

  it("rejects empty, zero, and unsafe integer values", () => {
    expect(isPositiveIntegerString("")).toBe(false);
    expect(isPositiveIntegerString("0")).toBe(false);
    expect(isPositiveIntegerString("400000")).toBe(true);
    expect(isPositiveIntegerString("9007199254740992")).toBe(false);
  });
});
