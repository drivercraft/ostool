export type TurnstileApi = {
  render: (element: HTMLElement, options: Record<string, unknown>) => string;
  remove: (id: string) => void;
};
let loading: Promise<TurnstileApi> | undefined;
const current = () =>
  (window as unknown as { turnstile?: TurnstileApi }).turnstile;

/** Load the official script once; network errors, missing APIs and timeouts remain retryable. */
export function loadTurnstile(): Promise<TurnstileApi> {
  const api = current();
  if (api && typeof api.render === "function") return Promise.resolve(api);
  if (loading) return loading;
  const pending = new Promise<TurnstileApi>((resolve, reject) => {
    const script = document.createElement("script");
    script.src =
      "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";
    script.async = true;
    const nonce = document.querySelector<HTMLMetaElement>(
      'meta[name="csp-nonce"]',
    )?.content;
    if (nonce) script.nonce = nonce;
    const fail = () => {
      clearTimeout(timer);
      script.remove();
      reject(new Error("Cloudflare 人机验证加载失败，请检查网络后重新验证。"));
    };
    const timer = window.setTimeout(fail, 15000);
    script.onerror = fail;
    script.onload = () => {
      const loaded = current();
      if (!loaded || typeof loaded.render !== "function") {
        fail();
        return;
      }
      clearTimeout(timer);
      resolve(loaded);
    };
    document.head.append(script);
  });
  loading = pending.catch((error) => {
    loading = undefined;
    throw error;
  });
  return loading;
}
