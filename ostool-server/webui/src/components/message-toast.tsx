import { useEffect } from "react";
import { toast } from "sonner";

/** Display state-driven feedback without adding a second inline message. */
export function MessageToast({
  message,
  kind = "error",
}: {
  message?: string | null | false;
  kind?: "error" | "success" | "warning" | "info";
}) {
  useEffect(() => {
    if (message) {
      // Shared subscriptions can report the same error in several components.
      toast[kind](message, { id: `${kind}:${message}` });
    }
  }, [message, kind]);
  return null;
}
