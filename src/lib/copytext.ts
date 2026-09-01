/** Write text to the system clipboard. WKWebView has no clipboard API outside a
 *  secure context, so a failure falls back to the old execCommand; both failing
 *  returns false — the caller is responsible for saying so rather than failing
 *  silently (the same stance as clipboard_copy_image's "the button reports the
 *  failure").
 *  It used to be an internal function of livepreview.ts and was lifted out once the
 *  file tree's "copy path" shared it. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    ta.remove();
    return ok;
  }
}
