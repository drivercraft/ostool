import type { BoardConfig } from "@/types/api";

export function normalizeBoardTypeForId(boardType: string): string {
  return boardType
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

export function suggestBoardId(boardType: string, boards: BoardConfig[], currentBoardId?: string): string {
  const prefix = normalizeBoardTypeForId(boardType);
  if (!prefix) {
    return "";
  }

  const matchingBoards = boards.filter((board) => board.id !== currentBoardId);
  let maxSuffix = 0;

  for (const board of matchingBoards) {
    const match = board.id.match(new RegExp(`^${escapeRegExp(prefix)}-(\\d+)$`));
    if (match) {
      maxSuffix = Math.max(maxSuffix, Number(match[1]));
    }
  }

  if (maxSuffix > 0) {
    return `${prefix}-${maxSuffix + 1}`;
  }

  const sameTypeCount = matchingBoards.filter((board) => board.board_type === boardType.trim()).length;
  return `${prefix}-${sameTypeCount + 1}`;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
