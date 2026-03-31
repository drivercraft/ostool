import { RuleEffect, type UISchemaElement } from "@jsonforms/core";

export const boardEditorUiSchema = {
  type: "VerticalLayout",
  elements: [
    {
      type: "Group",
      label: "基本信息",
      elements: [
        {
          type: "HorizontalLayout",
          elements: [
            {
              type: "Control",
              scope: "#/properties/board_type",
              label: "板型",
            },
            {
              type: "Control",
              scope: "#/properties/id",
              label: "板子 ID",
            },
          ],
        },
        {
          type: "HorizontalLayout",
          elements: [
            {
              type: "Control",
              scope: "#/properties/name",
              label: "显示名称",
            },
            {
              type: "Control",
              scope: "#/properties/tags_text",
              label: "标签",
            },
          ],
        },
        {
          type: "Control",
          scope: "#/properties/notes",
          label: "备注",
          options: {
            multi: true,
          },
        },
        {
          type: "Control",
          scope: "#/properties/disabled",
          label: "禁用该开发板",
        },
      ],
    },
    {
      type: "Group",
      label: "串口配置",
      elements: [
        {
          type: "Control",
          scope: "#/properties/serial_enabled",
          label: "启用串口",
        },
        {
          type: "HorizontalLayout",
          rule: {
            effect: RuleEffect.SHOW,
            condition: {
              scope: "#/properties/serial_enabled",
              schema: { const: true },
            },
          },
          elements: [
            {
              type: "Control",
              scope: "#/properties/serial_port",
              label: "串口设备",
            },
            {
              type: "Control",
              scope: "#/properties/serial_baud_rate",
              label: "波特率",
            },
          ],
        },
      ],
    },
    {
      type: "Group",
      label: "电源管理",
      elements: [
        {
          type: "Control",
          scope: "#/properties/power_management_enabled",
          label: "启用电源管理",
        },
        {
          type: "Control",
          scope: "#/properties/power_management_kind",
          label: "电源管理类型",
          rule: {
            effect: RuleEffect.SHOW,
            condition: {
              scope: "#/properties/power_management_enabled",
              schema: { const: true },
            },
          },
        },
        {
          type: "Group",
          label: "Custom 电源管理",
          rule: {
            effect: RuleEffect.SHOW,
            condition: {
              scope: "#",
              schema: {
                properties: {
                  power_management_enabled: { const: true },
                  power_management_kind: { const: "custom" },
                },
                required: ["power_management_enabled", "power_management_kind"],
              },
            },
          },
          elements: [
            {
              type: "Control",
              scope: "#/properties/power_management_custom/properties/power_on_cmd",
              label: "开机命令",
            },
            {
              type: "Control",
              scope: "#/properties/power_management_custom/properties/power_off_cmd",
              label: "关机命令",
            },
          ],
        },
        {
          type: "Group",
          label: "中盛继电模块",
          rule: {
            effect: RuleEffect.SHOW,
            condition: {
              scope: "#",
              schema: {
                properties: {
                  power_management_enabled: { const: true },
                  power_management_kind: { const: "zhongsheng_relay" },
                },
                required: ["power_management_enabled", "power_management_kind"],
              },
            },
          },
          elements: [
            {
              type: "Control",
              scope: "#/properties/power_management_zhongsheng_relay/properties/serial_port",
              label: "可用串口设备",
            },
          ],
        },
      ],
    },
    {
      type: "Group",
      label: "启动方式",
      elements: [
        {
          type: "Control",
          scope: "#/properties/boot_kind",
          label: "启动模式",
        },
      ],
    },
    {
      type: "Group",
      label: "U-Boot 启动配置",
      rule: {
        effect: RuleEffect.SHOW,
        condition: {
          scope: "#/properties/boot_kind",
          schema: { const: "uboot" },
        },
      },
      elements: [
        {
          type: "HorizontalLayout",
          elements: [
            {
              type: "Control",
              scope: "#/properties/uboot/properties/use_tftp",
              label: "使用 TFTP 启动",
            },
            {
              type: "Control",
              scope: "#/properties/uboot/properties/timeout",
              label: "超时（秒）",
            },
          ],
        },
        {
          type: "HorizontalLayout",
          elements: [
            {
              type: "Control",
              scope: "#/properties/uboot/properties/fit_load_addr",
              label: "FIT 加载地址",
            },
            {
              type: "Control",
              scope: "#/properties/uboot/properties/kernel_load_addr",
              label: "内核加载地址",
            },
          ],
        },
        {
          type: "HorizontalLayout",
          elements: [
            {
              type: "Control",
              scope: "#/properties/uboot/properties/shell_prefix",
              label: "Shell Prefix",
            },
            {
              type: "Control",
              scope: "#/properties/uboot/properties/shell_init_cmd",
              label: "Shell Init Command",
            },
          ],
        },
        {
          type: "Control",
          scope: "#/properties/uboot/properties/success_regex_text",
          label: "成功匹配（每行一条）",
          options: {
            multi: true,
          },
        },
        {
          type: "Control",
          scope: "#/properties/uboot/properties/fail_regex_text",
          label: "失败匹配（每行一条）",
          options: {
            multi: true,
          },
        },
        {
          type: "Control",
          scope: "#/properties/uboot/properties/uboot_cmd_text",
          label: "预置 U-Boot 命令（每行一条）",
          options: {
            multi: true,
          },
        },
      ],
    },
    {
      type: "Group",
      label: "PXE 占位配置",
      rule: {
        effect: RuleEffect.SHOW,
        condition: {
          scope: "#/properties/boot_kind",
          schema: { const: "pxe" },
        },
      },
      elements: [
        {
          type: "Control",
          scope: "#/properties/pxe/properties/notes",
          label: "说明",
          options: {
            multi: true,
          },
        },
      ],
    },
  ],
} satisfies UISchemaElement;
