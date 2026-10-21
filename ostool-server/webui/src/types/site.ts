export interface SiteSettings {
  name: string;
  short_name: string;
  tagline: string;
  description: string;
  logo_url: string;
  favicon_url: string;
  support_email: string;
  support_phone: string;
  support_url: string;
  operator: string;
  address: string;
  copyright: string;
  icp_number: string;
  icp_url: string;
  police_number: string;
  police_url: string;
  github_url: string;
  maintenance: boolean;
  maintenance_message: string;
  terms_title: string;
  terms_body: string;
  privacy_title: string;
  privacy_body: string;
}
export interface SiteView {
  revision: number;
  terms_version: number;
  privacy_version: number;
  updated_at: string;
  terms_published_at?: string;
  privacy_published_at?: string;
  settings: SiteSettings;
}
export interface LegalConsent {
  accepted: boolean;
  terms_version: number;
  privacy_version: number;
}
export interface LegalVersion {
  kind: "terms" | "privacy";
  version: number;
  title: string;
  content_hash: string;
  published_at: string;
}
export interface MailDelivery {
  id: string;
  purpose: string;
  recipient_email: string;
  status: "sending" | "sent" | "failed";
  error_code: string | null;
  created_at: string;
  completed_at: string | null;
}
