import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { useSiteResource } from "@/api/events";
import { api, type AuthConfig } from "@/api/client";
import { readAuthBootstrap } from "@/auth-session";
import { Skeleton } from "@/components/ui/skeleton";

const EditionContext = createContext<AuthConfig | undefined>(undefined);
export function EditionProvider({ children }: { children: ReactNode }) {
  const live = useSiteResource();
  const [config, setConfig] = useState<AuthConfig | undefined>(
    readAuthBootstrap,
  );
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    void api
      .authConfig()
      .then((value) => {
        if (active) setConfig(value);
      })
      .catch(() => {
        if (active) setError("无法读取站点版本配置，请刷新重试。");
      });
    return () => {
      active = false;
    };
  }, []);
  const value = useMemo(
    () => (config ? { ...config, ...live?.auth } : undefined),
    [config, live?.auth],
  );
  if (!value) return <StartupScreen error={error} />;
  return (
    <EditionContext.Provider value={value}>{children}</EditionContext.Provider>
  );
}
export function useEdition() {
  const config = useContext(EditionContext);
  if (!config) throw new Error("EditionProvider is required");
  return config;
}

export function StartupScreen({
  error = "",
  admin = window.location.pathname.startsWith("/admin"),
}: {
  error?: string;
  admin?: boolean;
}) {
  return (
    <div
      className={
        admin ? "startup-shell startup-admin" : "startup-shell startup-website"
      }
      aria-busy={!error}
    >
      {admin && (
        <aside>
          <strong>ostool</strong>
          <span>工作区</span>
          {Array.from({ length: 6 }, (_, index) => (
            <Skeleton key={index} className="h-8 w-full" />
          ))}
        </aside>
      )}
      <div className="startup-body">
        <header>
          <strong>ostool</strong>
          <span>{admin ? "管理后台" : "开发板协作平台"}</span>
        </header>
        <main>
          {error ? (
            <div role="alert">
              {error}
              <button type="button" onClick={() => window.location.reload()}>
                重试
              </button>
            </div>
          ) : (
            <>
              <Skeleton className="h-12 w-2/3" />
              <Skeleton className="h-6 w-1/2" />
              <div className="startup-metrics">
                {[0, 1, 2].map((key) => (
                  <Skeleton key={key} className="h-32 w-full" />
                ))}
              </div>
              <Skeleton className="min-h-64 flex-1 w-full" />
            </>
          )}
        </main>
        <footer>{error ? "服务暂时不可用" : "正在载入工作区…"}</footer>
      </div>
    </div>
  );
}
