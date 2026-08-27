import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import { useForm } from "react-hook-form";
import { Form, FormField, FormItem } from "@/components/ui/form";
import {
  ClassifierRoutingField,
  DEFAULT_CLASSIFIER_ROUTING,
} from "@/components/providers/forms/ClassifierRoutingField";
import type { ClassifierRoutingConfig } from "@/types";

function Harness() {
  const form = useForm({ defaultValues: { classifier: "" } });
  const [value, setValue] = useState<ClassifierRoutingConfig>({
    ...DEFAULT_CLASSIFIER_ROUTING,
  });

  return (
    <Form {...form}>
      <FormField
        control={form.control}
        name="classifier"
        render={() => (
          <FormItem>
            <ClassifierRoutingField value={value} onChange={setValue} />
          </FormItem>
        )}
      />
    </Form>
  );
}

describe("ClassifierRoutingField", () => {
  it("adds models and keeps the first model as the default", () => {
    render(<Harness />);

    fireEvent.change(screen.getByPlaceholderText("模型 ID，例如 gpt-5.6-sol"), {
      target: { value: "gpt-5.6-sol" },
    });
    fireEvent.change(screen.getByPlaceholderText("备注（可选）"), {
      target: { value: "稳定" },
    });
    fireEvent.click(screen.getByRole("button", { name: "添加" }));

    expect(screen.getByText("gpt-5.6-sol")).toBeInTheDocument();
    expect(screen.getByText("默认")).toBeInTheDocument();
  });

  it("supports enabling the router and selecting a strategy", () => {
    render(<Harness />);

    fireEvent.click(screen.getByText("启用分类器分流"));
    fireEvent.click(screen.getByRole("radio", { name: /最便宜优先/ }));

    expect(screen.getByText("最便宜优先")).toBeInTheDocument();
  });

  it("supports direct external URLs and requires a Claude Code restart", () => {
    render(<Harness />);

    fireEvent.click(screen.getByText("启用分类器分流"));

    expect(
      screen.getByText(
        "直连外部 API URL 时通过 Claude Code 的 CLAUDE_CODE_AUTO_MODE_MODEL 生效，无需本地代理；代理模式仍按请求分流。",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "保存当前供应商后，请完全重启正在运行的 Claude Code；已启动的会话不会重新读取分类器模型环境变量。",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText("请至少添加一个可用的分类器模型后再保存。"),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /本地代理|接管 Claude/ }),
    ).not.toBeInTheDocument();

    fireEvent.change(screen.getByPlaceholderText("模型 ID，例如 gpt-5.6-sol"), {
      target: { value: "classifier-model" },
    });
    fireEvent.click(screen.getByRole("button", { name: "添加" }));

    expect(
      screen.queryByText("请至少添加一个可用的分类器模型后再保存。"),
    ).not.toBeInTheDocument();
  });
});
