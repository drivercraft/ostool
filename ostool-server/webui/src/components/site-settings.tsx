import { createContext, useContext, useEffect, type ReactNode } from "react";
import { site, useSiteResource } from "@/api/events";
import { useEdition } from "@/edition";
import type { SiteView } from "@/types/api";
const fallback: SiteView = {
  revision: 1,
  terms_version: 1,
  privacy_version: 1,
  updated_at: "",
  settings: {
    name: "ostool",
    short_name: "ostool",
    tagline: "BOARD PLATFORM",
    description: "为嵌入式团队打造的开发板协作平台。",
    logo_url: "",
    favicon_url: "",
    support_email: "",
    support_phone: "",
    support_url: "",
    operator: "",
    address: "",
    copyright: "ostool. Built for hardware teams.",
    icp_number: "",
    icp_url: "",
    police_number: "",
    police_url: "",
    github_url: "https://github.com/drivercraft/ostool",
    maintenance: false,
    maintenance_message: "",
    terms_title: "用户协议",
    terms_body: "",
    privacy_title: "隐私政策",
    privacy_body: "",
  },
};
const Context = createContext<SiteView>(fallback);
export const useSiteSettings = () => useContext(Context);
export function SiteSettingsProvider({ children }: { children: ReactNode }) {
  const initial = useEdition().site;
  const live = useSiteResource();
  const value = live ?? initial ?? fallback;
  useEffect(() => {
    site.connect();
    return site.disconnect;
  }, []);
  useEffect(() => {
    let meta = document.querySelector<HTMLMetaElement>(
      'meta[name="description"]',
    );
    if (!meta) {
      meta = document.createElement("meta");
      meta.name = "description";
      document.head.append(meta);
    }
    meta.content = value.settings.description;
    let icon = document.querySelector<HTMLLinkElement>("link[data-site-icon]");
    if (value.settings.favicon_url) {
      if (!icon) {
        icon = document.createElement("link");
        icon.rel = "icon";
        icon.dataset.siteIcon = "true";
        document.head.append(icon);
      }
      icon.href = value.settings.favicon_url;
    } else {
      icon?.remove();
    }
  }, [value.settings.description, value.settings.favicon_url]);
  return <Context.Provider value={value}>{children}</Context.Provider>;
}
