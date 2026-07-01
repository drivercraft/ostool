export type LeaseCalendarSlotBase = {
  startIso: string;
  endIso: string;
};

export type LeaseCalendarSelection = {
  startIso: string;
  endIso: string;
};

export function toDatetimeLocal(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "";
  }
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60000);
  return local.toISOString().slice(0, 16);
}

export function fromDatetimeLocal(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "" : date.toISOString();
}

export function windowsOverlap(startA: string, endA: string, startB: string, endB: string) {
  if (!startA || !endA || !startB || !endB) {
    return false;
  }
  return new Date(startA).getTime() < new Date(endB).getTime()
    && new Date(endA).getTime() > new Date(startB).getTime();
}

export function slotOverlapsSelection(
  slot: LeaseCalendarSlotBase,
  selectionStartIso: string,
  selectionEndIso: string,
) {
  return windowsOverlap(slot.startIso, slot.endIso, selectionStartIso, selectionEndIso);
}

function selectionFromSlots<TSlot extends LeaseCalendarSlotBase>(
  slots: TSlot[],
  firstIndex: number,
  lastIndex: number,
): LeaseCalendarSelection {
  return {
    startIso: slots[firstIndex].startIso,
    endIso: slots[lastIndex].endIso,
  };
}

export function selectLeaseCalendarRange<TSlot extends LeaseCalendarSlotBase>(
  slots: TSlot[],
  clickedSlot: TSlot,
  selectionStartIso: string,
  selectionEndIso: string,
  isSlotSelectable: (slot: TSlot) => boolean,
): LeaseCalendarSelection | null {
  const clickedIndex = slots.findIndex((slot) => slot.startIso === clickedSlot.startIso && slot.endIso === clickedSlot.endIso);
  if (clickedIndex < 0 || !isSlotSelectable(clickedSlot)) {
    return null;
  }

  const selectedIndexes = slots
    .map((slot, index) => ({ slot, index }))
    .filter(({ slot }) => slotOverlapsSelection(slot, selectionStartIso, selectionEndIso))
    .map(({ index }) => index);

  if (selectedIndexes.length === 0) {
    return selectionFromSlots(slots, clickedIndex, clickedIndex);
  }

  const firstSelectedIndex = Math.min(...selectedIndexes);
  const lastSelectedIndex = Math.max(...selectedIndexes);

  if (selectedIndexes.includes(clickedIndex)) {
    if (clickedIndex === firstSelectedIndex) {
      return { startIso: "", endIso: "" };
    }
    return selectionFromSlots(slots, firstSelectedIndex, clickedIndex - 1);
  }

  const nextFirstIndex = Math.min(firstSelectedIndex, clickedIndex);
  const nextLastIndex = Math.max(lastSelectedIndex, clickedIndex);
  const nextRange = slots.slice(nextFirstIndex, nextLastIndex + 1);
  if (nextRange.every(isSlotSelectable)) {
    return selectionFromSlots(slots, nextFirstIndex, nextLastIndex);
  }

  return selectionFromSlots(slots, clickedIndex, clickedIndex);
}
