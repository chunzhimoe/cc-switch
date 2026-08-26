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
});
