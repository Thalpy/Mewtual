/** Return a normalized HTTP(S) URL that is safe to hand to an image element. */
export function safeRemoteUrl(raw: string): string {
  try {
    const u = new URL(raw);
    return (u.protocol === "https:" || u.protocol === "http:") && raw.length <= 4096 ? u.href : "";
  } catch {
    return "";
  }
}

/**
 * Recognise links that can be displayed as images without fetching metadata first.
 * Ordinary links remain links; common Giphy share URLs are mapped to their media CDN.
 */
export function pastedImageUrl(raw: string): string {
  const safe = safeRemoteUrl(raw);
  if (!safe) return "";
  const u = new URL(safe);
  if (/\.(?:png|jpe?g|gif|webp|avif)(?:$|[?#])/i.test(u.pathname + u.search + u.hash)) return safe;
  if (u.hostname === "giphy.com" || u.hostname === "www.giphy.com") {
    const tail = u.pathname.split("/").filter(Boolean).pop() ?? "";
    const id = (tail.split("-").pop() ?? "").replace(/[^a-z0-9]/gi, "");
    if (id) return `https://media.giphy.com/media/${id}/giphy.gif`;
  }
  return "";
}
