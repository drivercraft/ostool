import type {
  CreateLeaseRequest,
  LeaseResponse,
  LeasesResponse,
  UserPasswordUpdateRequest,
} from "@/types/api";

import { request } from "./http";

export const userApi = {
  listUserLeases() {
    return request<LeasesResponse>("/api/v1/user/leases");
  },
  listUserLeaseAvailability() {
    return request<LeasesResponse>("/api/v1/user/leases/availability");
  },
  updateUserPassword(payload: UserPasswordUpdateRequest) {
    return request<void>("/api/v1/user/password", {
      method: "POST",
      bodyJson: payload,
    });
  },
  createLease(payload: CreateLeaseRequest) {
    return request<LeaseResponse>("/api/v1/user/leases", {
      method: "POST",
      bodyJson: payload,
    });
  },
  deleteLease(leaseId: string) {
    return request<void>(`/api/v1/user/leases/${encodeURIComponent(leaseId)}`, {
      method: "DELETE",
    });
  },
  heartbeatLease(leaseId: string) {
    return request<LeaseResponse>(
      `/api/v1/user/leases/${encodeURIComponent(leaseId)}/heartbeat`,
      { method: "POST" },
    );
  },
};
