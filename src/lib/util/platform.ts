/**
 * Platform helpers — Agency Agents ships on macOS, Linux, and Windows 11, so
 * the UI must not hardcode Mac-only affordances. The most visible one is the
 * keyboard-shortcut glyph: ⌘ on macOS, "Ctrl" everywhere else. The matching
 * keydown handlers already accept both (metaKey || ctrlKey), so this only
 * governs what we DISPLAY.
 *
 * Detection is synchronous (reads `navigator` in the WebView) so it can be used
 * directly in component markup without an async Tauri round-trip. It degrades
 * to non-Mac on SSR / when `navigator` is absent.
 */

/** True when running on macOS (Cmd-key platform). */
export const isMac: boolean = (() => {
  if (typeof navigator === "undefined") return false;
  // `userAgentData.platform` is the modern, non-deprecated source; fall back to
  // the classic `platform` / userAgent strings for WebViews that lack it.
  const uaPlatform =
    (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData?.platform ??
    navigator.platform ??
    navigator.userAgent ??
    "";
  return /mac/i.test(uaPlatform);
})();

/** Display label for the primary modifier key: "⌘" on macOS, "Ctrl" elsewhere. */
export const modKey: string = isMac ? "⌘" : "Ctrl";

/**
 * Render a shortcut for display, given the key(s) after the modifier. On macOS
 * we concatenate tightly (⌘K); elsewhere we join with "+" (Ctrl+K), matching
 * each platform's convention.
 *
 *   shortcut("K")        → "⌘K"    | "Ctrl+K"
 *   shortcut("⇧", "L")   → "⌘⇧L"   | "Ctrl+Shift+L"  (caller passes nice tokens)
 */
export function shortcut(...keys: string[]): string {
  return isMac ? `${modKey}${keys.join("")}` : `${modKey}+${keys.join("+")}`;
}
