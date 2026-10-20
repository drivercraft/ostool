import {
  createContext,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import { api, type AuthConfig } from "@/api/client";
import { Skeleton } from "@/components/ui/skeleton";

const EditionContext = createContext<AuthConfig | undefined>(undefined);
export function EditionProvider({ children }: { children: ReactNode }) {
  const [config, setConfig] = useState<AuthConfig>();
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
  if (error)
    return (
      <div className="admin-gate" role="alert">
        {error}
      </div>
    );
  if (!config)
    return (
      <div className="admin-gate">
        <Skeleton className="h-20 w-80" />
      </div>
    );
  return (
    <EditionContext.Provider value={config}>{children}</EditionContext.Provider>
  );
}
export function useEdition() {
  const config = useContext(EditionContext);
  if (!config) throw new Error("EditionProvider is required");
  return config;
}
