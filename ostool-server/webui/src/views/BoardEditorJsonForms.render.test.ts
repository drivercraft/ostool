import { mount } from "@vue/test-utils";
import { JsonForms } from "@jsonforms/vue";
import { vanillaRenderers } from "@jsonforms/vue-vanilla";
import { describe, expect, it } from "vitest";
import { defineComponent, h, markRaw } from "vue";

import type { BoardEditorDocument } from "@/types/api";
import { jsonFormsAjv } from "@/utils/jsonFormsAjv";
import { boardEditorUiSchema } from "@/utils/boardEditorUiSchema";

function createDocument(): BoardEditorDocument {
  return {
    data: {
      id: "phytiumpi",
      name: "phytiumpi",
      board_type: "phytiumpi",
      tags_text: "",
      notes: "",
      disabled: false,
      serial_enabled: true,
      serial_port: "/dev/ttyUSB0",
      serial_baud_rate: 115200,
      power_management_enabled: true,
      power_management_kind: "zhongsheng_relay",
      power_management_custom: {
        power_on_cmd: "",
        power_off_cmd: "",
      },
      power_management_zhongsheng_relay: {
        serial_port: "/dev/ttyUSB1",
      },
      boot_kind: "uboot",
      uboot: {
        use_tftp: false,
        kernel_load_addr: "",
        fit_load_addr: "",
        success_regex_text: "",
        fail_regex_text: "",
        uboot_cmd_text: "",
        shell_prefix: "",
        shell_init_cmd: "",
        timeout: null,
      },
      pxe: {
        notes: "",
      },
    },
    schema: ({
      $schema: "https://json-schema.org/draft/2020-12/schema",
      type: "object",
      properties: {
        id: { type: "string", minLength: 1 },
        name: { type: "string", minLength: 1 },
        board_type: { type: "string", minLength: 1 },
        tags_text: { type: "string", default: "" },
        notes: { type: "string", default: "" },
        disabled: { type: "boolean", default: false },
        serial_enabled: { type: "boolean", default: false },
        serial_port: {
          type: "string",
          oneOf: [
            { const: "/dev/ttyUSB0", title: "/dev/ttyUSB0" },
            { const: "/dev/ttyUSB9", title: "/dev/ttyUSB9 (当前配置，未检测到)" },
          ],
        },
        serial_baud_rate: { type: "integer", format: "uint32", minimum: 1 },
        power_management_enabled: { type: "boolean", default: false },
        power_management_kind: {
          type: "string",
          enum: ["custom", "zhongsheng_relay"],
          default: "custom",
        },
        power_management_custom: { $ref: "#/$defs/BoardEditorCustomPowerManagementData" },
        power_management_zhongsheng_relay: {
          $ref: "#/$defs/BoardEditorZhongshengRelayPowerManagementData",
        },
        boot_kind: { type: "string", enum: ["uboot", "pxe"], default: "uboot" },
        uboot: { $ref: "#/$defs/BoardEditorUbootData" },
        pxe: { $ref: "#/$defs/BoardEditorPxeData" },
      },
      $defs: {
        BoardEditorCustomPowerManagementData: {
          type: "object",
          properties: {
            power_on_cmd: { type: "string", default: "" },
            power_off_cmd: { type: "string", default: "" },
          },
        },
        BoardEditorZhongshengRelayPowerManagementData: {
          type: "object",
          properties: {
            serial_port: {
              type: "string",
              oneOf: [
                { const: "/dev/ttyUSB1", title: "/dev/ttyUSB1" },
                { const: "/dev/ttyUSB7", title: "/dev/ttyUSB7 (当前配置，未检测到)" },
              ],
            },
          },
        },
        BoardEditorUbootData: {
          type: "object",
          properties: {
            use_tftp: { type: "boolean", default: false },
            kernel_load_addr: { type: "string", default: "" },
            fit_load_addr: { type: "string", default: "" },
            success_regex_text: { type: "string", default: "" },
            fail_regex_text: { type: "string", default: "" },
            uboot_cmd_text: { type: "string", default: "" },
            shell_prefix: { type: "string", default: "" },
            shell_init_cmd: { type: "string", default: "" },
            timeout: { type: ["integer", "null"], format: "uint64" },
          },
        },
        BoardEditorPxeData: {
          type: "object",
          properties: {
            notes: { type: "string", default: "" },
          },
        },
      },
      required: ["id", "name", "board_type"],
    }) as BoardEditorDocument["schema"],
  };
}

describe("BoardEditor JSON Forms", () => {
  it("renders the power management editor with relay serial schema", () => {
    const document = createDocument();

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
    expect(wrapper.html()).toContain("电源管理");
    expect(wrapper.html()).toContain("中盛继电模块");
    expect(wrapper.html()).toContain("可用串口设备");
  });
});
