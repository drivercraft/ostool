import { useAccountResource } from "@/api/events";
import { useSiteSession } from "@/apps/web/session";

/** Eligibility is personal; use the authenticated catalog, never the public one. */
export function BoardName({ id, name }: { id: string; name?: string }) {
  const { user } = useSiteSession();
  const catalog = useAccountResource("catalog");
  const eligible =
    (user?.is_admin || user?.permissions?.includes("board.instant")) &&
    catalog?.some((b) => b.id === id && b.auto_approval_available);
  return (
    <>
      {name || id}
      {eligible ? "（免审批）" : ""}
    </>
  );
}
