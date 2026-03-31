import { mount } from "@vue/test-utils";
import { JsonForms } from "@jsonforms/vue";
import { vanillaRenderers } from "@jsonforms/vue-vanilla";
import { describe, expect, it } from "vitest";
import { defineComponent, h, markRaw } from "vue";

import { jsonFormsAjv } from "@/utils/jsonFormsAjv";
import { boardEditorUiSchema } from "@/utils/boardEditorUiSchema";

describe("BoardEditor JSON Forms", () => {
  it("renders controls with the live board editor document shape", async () => {
    const response = await fetch("http://127.0.0.1:8080/api/v1/admin/boards/phytiumpi");
    const document = await response.json();

    const App = defineComponent({
      render() {
        return h(JsonForms, {
          data: document.data,
          schema: document.schema,
          uischema: boardEditorUiSchema,
          renderers: markRaw(vanillaRenderers),
          ajv: jsonFormsAjv,
          validationMode: "ValidateAndShow",
          onChange: () => {},
        });
      },
    });

    const wrapper = mount(App);
    expect(wrapper.html()).toContain("基本信息");
    expect(wrapper.html()).toContain("板型");
  });
});
