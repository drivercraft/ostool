import { describe, expect, it } from "vitest";

import { boardToForm, createDefaultBoardForm, formToBoard, parseTags } from "./boardForm";

describe("board form helpers", () => {
  it("parses tags from comma separated text", () => {
    expect(parseTags(" rk3568, lab , , usb ")).toEqual(["rk3568", "lab", "usb"]);
  });

  it("converts between form and board payload", () => {
    const form = createDefaultBoardForm();
    form.id = "demo-board";
    form.name = "Demo";
    form.board_type = "rk3568";
    form.tagsText = "lab, usb";
    form.serialEnabled = true;
    form.serialPort = "/dev/ttyUSB0";
    form.serialBaudRate = 1500000;
    form.uboot.interface = "eth0";
    form.uboot.server_ip_override = "192.168.1.10";
    form.uboot.success_regex_text = "booted\nlogin:";

    const board = formToBoard(form);
    expect(board.id).toBe("demo-board");
    expect(board.serial?.baud_rate).toBe(1500000);
    expect(board.boot.kind).toBe("uboot");

    const roundTrip = boardToForm(board);
    expect(roundTrip.tagsText).toBe("lab, usb");
    expect(roundTrip.uboot.success_regex_text).toBe("booted\nlogin:");
  });
});
