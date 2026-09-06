import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import { useForm } from "react-hook-form";
import { Form, FormField, FormItem } from "@/components/ui/form";
import { AutoClassifierBypassField } from "@/components/providers/forms/AutoClassifierBypassField";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import zh from "@/i18n/locales/zh.json";
import zhTW from "@/i18n/locales/zh-TW.json";

function Harness({ classifierRoutingConfigured = true }) {
  const form = useForm({ defaultValues: { bypass: false } });
  const [enabled, setEnabled] = useState(false);
  return (
    <Form {...form}>
      <FormField
        control={form.control}
        name="bypass"
        render={() => (
          <FormItem>
            <AutoClassifierBypassField
              value={enabled}
              onChange={setEnabled}
              classifierRoutingConfigured={classifierRoutingConfigured}
            />
          </FormItem>
        )}
      />
    </Form>
  );
}

describe("AutoClassifierBypassField", () => {
  it.each([
    { locale: "en", messages: en },
    { locale: "ja", messages: ja },
    { locale: "zh", messages: zh },
    { locale: "zh-TW", messages: zhTW },
  ])("discloses the prompt setting in $locale", ({ messages }) => {
    expect(messages.providerForm.skipAutoClassifierDescription).toContain(
      "skipDangerousModePermissionPrompt=true",
    );
    expect(messages.providerForm.skipAutoClassifierConfirmMessage).toContain(
      "skipDangerousModePermissionPrompt = true",
    );
  });

  it("requires destructive confirmation before enabling", () => {
    render(<Harness />);

    const toggle = screen.getByRole("switch");
    expect(toggle).not.toBeChecked();

    fireEvent.click(toggle);
    expect(screen.getByText("确认跳过 Auto 分类器？")).toBeInTheDocument();
    expect(
      screen.getByText(/skipDangerousModePermissionPrompt = true/),
    ).toBeInTheDocument();
    expect(toggle).not.toBeChecked();

    fireEvent.click(screen.getByRole("button", { name: "common.cancel" }));
    expect(
      screen.queryByText("确认跳过 Auto 分类器？"),
    ).not.toBeInTheDocument();
    expect(toggle).not.toBeChecked();
  });

  it("shows persistent risk and routing-paused warnings after confirmation", () => {
    render(<Harness />);

    fireEvent.click(screen.getByRole("switch"));
    fireEvent.click(screen.getByRole("button", { name: "仍要启用" }));

    expect(screen.getByRole("switch")).toBeChecked();
    expect(
      screen.getByText(
        "该设置会绕过 Auto 模式的逐操作安全分类。保存后必须完全重启 Claude Code；请仅在你信任任务内容并接受 bypassPermissions 风险时启用。",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "已保存的分类器分流配置会继续保留，但跳过开关启用期间不会生效；关闭后自动恢复。",
      ),
    ).toBeInTheDocument();
  });

  it("disables immediately without another confirmation", () => {
    render(<Harness classifierRoutingConfigured={false} />);

    fireEvent.click(screen.getByRole("switch"));
    fireEvent.click(screen.getByRole("button", { name: "仍要启用" }));
    fireEvent.click(screen.getByRole("switch"));

    expect(screen.getByRole("switch")).not.toBeChecked();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
