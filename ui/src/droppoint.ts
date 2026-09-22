// OS 드롭 좌표·진단의 순수 판단 로직 (DOM 무접촉 — main.ts가 이벤트와 화면에 배선한다).
// macOS·Linux payload.position은 논리 px이므로 그대로, Windows만 물리 px이므로 /dpr 한다.
// 근거: wry 0.55.1 wkwebview/drag_drop.rs:85-87은 draggingLocation(포인트)의 y만 뒤집고
// backingScaleFactor를 곱하지 않는다. Linux(webkitgtk)도 GTK 논리 좌표 그대로다.
// Windows는 wry 0.55.1 webview2/drag_drop.rs:167의 ScreenToClient 물리 좌표를 쓴다.
// tauri-runtime-wry 2.11.2 lib.rs:4866-4882의 PhysicalPosition은 이름만으로 단위를 보장하지 않는다.
// tauri#10744의 잔여 y 오프셋은 드롭 대상 하이라이트·진단으로 확인한다.

export type DropPlatform = "macos" | "windows" | "linux";
type DropPoint = { x: number; y: number };

export function dropPlatformFromUserAgent(ua: string): DropPlatform {
  if (/Windows/i.test(ua)) return "windows";
  return /Macintosh|Mac OS X/i.test(ua) ? "macos" : "linux";
}

export function dropPointToCss(pos: DropPoint, dpr: number, platform: DropPlatform): DropPoint {
  const scale = Number.isFinite(dpr) && dpr > 0 ? dpr : 1;
  return platform === "windows"
    ? { x: pos.x / scale, y: pos.y / scale }
    : { x: pos.x, y: pos.y };
}

export function dropDebugDetail(raw: DropPoint, dpr: number, platform: DropPlatform, css: DropPoint): string {
  return `raw=(${String(raw.x)}, ${String(raw.y)}) · dpr=${String(dpr)} · ${platform} · CSS=(${String(css.x)}, ${String(css.y)})`;
}

export function isDropDebugEnabled(stored: string | null | undefined): boolean {
  return stored === "1";
}
