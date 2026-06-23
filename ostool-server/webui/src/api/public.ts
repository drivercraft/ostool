import type { BoardTypeSummary } from "@/types/api";

import { request } from "./http";

export const publicApi = {
  listBoardTypes() {
    return request<BoardTypeSummary[]>("/api/v1/board-types");
  },
};
