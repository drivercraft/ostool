// Preserve partial drafts while editing the display name and address separately.
export function splitMailSender(value: string | null) {
  const text = value?.trim() ?? "";
  const match = /^(.*)<([^<>]*)>$/.exec(text);
  if (!match) return { name: "", address: text };
  let name = match[1].trim();
  if (name.startsWith('"') && name.endsWith('"')) {
    try {
      name = JSON.parse(name);
    } catch {
      name = name.slice(1, -1);
    }
  }
  return { name, address: match[2].trim() };
}
export function formatMailSender(name: string, address: string) {
  if (!name && !address) return null;
  return name ? `${JSON.stringify(name)} <${address.trim()}>` : address.trim();
}
