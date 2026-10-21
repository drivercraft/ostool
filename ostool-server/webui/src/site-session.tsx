import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { api, type AuthUser } from "@/api/client";
import { account, useAccountResource } from "@/api/events";
import { useEdition } from "@/edition";

const SessionContext = createContext<{
  user?: AuthUser;
  checking: boolean;
  error: string;
}>({ checking: true, error: "" });
export function SiteSessionProvider({ children }: { children: ReactNode }) {
  const publicEdition = useEdition().edition === "public";
  const [session, setSession] = useState<{
    user?: AuthUser;
    checking: boolean;
    error: string;
  }>({ checking: true, error: "" });
  const profile = useAccountResource("profile");
  const connectedToken = useRef<string | null>(null);
  useEffect(() => {
    let generation = 0;
    let active = true;
    const load = async () => {
      const current = ++generation;
      if (!publicEdition || !localStorage.getItem("ostool.access_token")) {
        connectedToken.current = null;
        account.reset();
        setSession({ checking: false, error: "" });
        return;
      }
      try {
        const user = await api.me();
        if (active && current === generation) {
          if (account.get("profile")?.id !== user.id) account.reset();
          const token = localStorage.getItem("ostool.access_token");
          if (connectedToken.current !== token) account.disconnect();
          connectedToken.current = token;
          setSession({ user, checking: false, error: "" });
          account.connect();
        }
      } catch (error) {
        if (active && current === generation) {
          account.reset();
          setSession({
            checking: false,
            error: error instanceof Error ? error.message : "无法读取登录状态",
          });
        }
      }
    };
    const changed = (event: Event) => {
      if (
        event instanceof StorageEvent &&
        event.key !== null &&
        event.key !== "ostool.access_token"
      )
        return;
      void load();
    };
    void load();
    window.addEventListener("ostool:auth", changed);
    window.addEventListener("storage", changed);
    return () => {
      active = false;
      ++generation;
      window.removeEventListener("ostool:auth", changed);
      window.removeEventListener("storage", changed);
      account.disconnect();
    };
  }, [publicEdition]);
  return (
    <SessionContext.Provider
      value={{
        ...session,
        user: session.user ? (profile ?? session.user) : undefined,
      }}
    >
      {children}
    </SessionContext.Provider>
  );
}
export const useSiteSession = () => useContext(SessionContext);
