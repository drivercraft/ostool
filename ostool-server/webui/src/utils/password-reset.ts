/** Reset credentials travel in the fragment, outside HTTP URLs and referrers. */
export function parsePasswordResetLink(hash: string) {
  const fields = new URLSearchParams(hash.replace(/^#/, ""));
  const challenge_id = fields.get("challenge_id") ?? "";
  const code = fields.get("code") ?? "";
  if (
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
      challenge_id,
    ) ||
    !/^[A-Za-z0-9_-]{43}$/.test(code)
  )
    return null;
  return { challenge_id, code };
}
