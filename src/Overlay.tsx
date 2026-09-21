import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { getActivitySnapshot, getOverlaySettings, getQuotaSnapshot, getUpdateStatus, hideResetPanel, openReleasePage, OPACITY_CHANGED, refreshQuotas, subscribeActivity, subscribeQuota, subscribeUpdateStatus, toggleResetPanel } from "./bridge";
import { QuotaRow } from "./QuotaRow";
import type { AgentActivity, ProviderActivity, ProviderId, ProviderQuota, QuotaPool, UpdateStatus } from "./types";
const providerOrder: ProviderId[] = ["chatgpt_work", "claude_code", "antigravity"];
const providerNames: Record<ProviderId, string> = { chatgpt_work: "ChatGPT Work", claude_code: "Claude Code", antigravity: "Antigravity" };
/** Long enough not to fire while aiming a click or starting a window drag. */
const LONG_PRESS_MS = 450;
/** Past this much travel the press is a drag, not a hold. */
const LONG_PRESS_TOLERANCE = 6;
const unavailablePool = (name: string | null = null): QuotaPool => ({ name, fiveHour: { remainingPercent: null, resetAt: null }, weekly: { remainingPercent: null, resetAt: null } });
function poolsFor(provider: ProviderId, pools?: QuotaPool[]): QuotaPool[] { return provider === "antigravity" ? ["Gemini", "Claude/GPT"].map((name) => pools?.find((pool) => pool.name === name) ?? unavailablePool(name)) : pools?.length ? pools : [unavailablePool()]; }
function orderedQuotas(snapshot: ProviderQuota[]): ProviderQuota[] { return providerOrder.map((provider) => { const entry = snapshot.find((item) => item.provider === provider); return { provider, pools: poolsFor(provider, entry?.pools), resetCredits: entry?.resetCredits ?? null }; }); }
export function Overlay() {
  const [quotas, setQuotas] = useState<ProviderQuota[]>(() => orderedQuotas([])); const [activities, setActivities] = useState<ProviderActivity[]>([]); const [opacity, setOpacity] = useState(88); const [update, setUpdate] = useState<UpdateStatus | null>(null);
  const opacityRef = useRef(opacity); const hudRef = useRef<HTMLElement>(null); const expiredKeys = useRef(new Set<string>()); opacityRef.current = opacity;
  // The panel lives in its own window, so the overlay tracks only whether its
  // own gesture opened one; the backend stays the authority on the toggle.
  const panelOpen = useRef(false); const press = useRef({ timer: 0, x: 0, y: 0, fired: false });
  useEffect(() => { if (!isTauri()) return; let active = true; const stop: Array<() => void> = []; const guard = (fn: () => void) => active ? stop.push(fn) : fn(); const initialize = async () => { try { guard(await subscribeQuota((value) => active && setQuotas(orderedQuotas(value)))); } catch {} try { guard(await subscribeActivity((value) => active && setActivities(value))); } catch {} try { guard(await listen<number>(OPACITY_CHANGED, ({ payload }) => active && setOpacity(payload))); } catch {} try { guard(await subscribeUpdateStatus((value) => active && setUpdate(value))); } catch {} try { const [settings, quotaSnapshot, activitySnapshot] = await Promise.all([getOverlaySettings(), getQuotaSnapshot(), getActivitySnapshot()]); if (active) { setOpacity(settings.opacity); setQuotas(orderedQuotas(quotaSnapshot)); setActivities(activitySnapshot); } } catch {} try { const status = await getUpdateStatus(); if (active) setUpdate(status); } catch {} }; void initialize(); return () => { active = false; stop.forEach((fn) => fn()); }; }, []);
  const closePanel = useCallback(() => { if (!panelOpen.current) return; panelOpen.current = false; if (isTauri()) void hideResetPanel().catch(() => undefined); }, []);
  const cancelPress = useCallback(() => { if (press.current.timer) { window.clearTimeout(press.current.timer); press.current.timer = 0; } }, []);
  useEffect(() => { if (!isTauri()) return; let timer: number | undefined; let unlisten: (() => void) | undefined; const appWindow = getCurrentWindow(); const onMoved = () => { cancelPress(); closePanel(); window.clearTimeout(timer); timer = window.setTimeout(() => { void invoke("persist_overlay_position"); }, 240); }; void appWindow.onMoved(onMoved).then((stop) => { unlisten = stop; }).catch(() => undefined); return () => { if (timer) window.clearTimeout(timer); unlisten?.(); }; }, [cancelPress, closePanel]);
  useEffect(() => { const target = hudRef.current; if (!target) return; const wheel = (event: WheelEvent) => { if (!event.ctrlKey) return; event.preventDefault(); const next = Math.max(30, Math.min(100, opacityRef.current + (event.deltaY > 0 ? -5 : 5))); if (next === opacityRef.current) return; setOpacity(next); if (isTauri()) void invoke<number>("set_overlay_opacity", { opacity: next }); }; target.addEventListener("wheel", wheel, { passive: false }); return () => target.removeEventListener("wheel", wheel); }, []);
  const onExpired = useCallback((provider: ProviderId, resetAt: string) => { const key = `${provider}:${resetAt}`; if (expiredKeys.current.has(key)) return; expiredKeys.current.add(key); void refreshQuotas(false, provider); }, []);
  const onContextMenu = useCallback((event: React.MouseEvent) => { event.preventDefault(); cancelPress(); if (isTauri()) void invoke("show_overlay_context_menu"); }, [cancelPress]);
  // A press that never moves opens the panel; a press that drags the window, a
  // release, or a display change all cancel it first.
  const onPressStart = useCallback((event: React.PointerEvent) => {
    if (event.button !== 0 || !isTauri()) return;
    cancelPress();
    press.current = { timer: window.setTimeout(() => { press.current.timer = 0; press.current.fired = true; void toggleResetPanel().then((open) => { panelOpen.current = open; }).catch(() => undefined); }, LONG_PRESS_MS), x: event.clientX, y: event.clientY, fired: false };
  }, [cancelPress]);
  const onPressMove = useCallback((event: React.PointerEvent) => { if (!press.current.timer) return; if (Math.abs(event.clientX - press.current.x) > LONG_PRESS_TOLERANCE || Math.abs(event.clientY - press.current.y) > LONG_PRESS_TOLERANCE) cancelPress(); }, [cancelPress]);
  // A plain click anywhere dismisses an open panel. The press that opened it is
  // excluded, so the panel does not close on the same release that opened it.
  const onClick = useCallback(() => { if (press.current.fired) { press.current.fired = false; return; } closePanel(); }, [closePanel]);
  const stateByProvider = useMemo(() => new Map(activities.map(({ provider, state }) => [provider, state])), [activities]);
  const longPress = useMemo(() => ({ onPointerDown: onPressStart, onPointerMove: onPressMove, onPointerUp: cancelPress, onPointerCancel: cancelPress, onPointerLeave: cancelPress }), [cancelPress, onPressMove, onPressStart]);
  return <main ref={hudRef} className="hud" style={{ opacity: opacity / 100 }} onContextMenu={onContextMenu} onClick={onClick} data-tauri-drag-region aria-label="LLM quota overlay"><div className="surface" data-tauri-drag-region>{quotas.map((quota) => <ProviderSection key={quota.provider} quota={quota} activity={stateByProvider.get(quota.provider) ?? "unknown"} onExpired={onExpired} longPress={quota.provider === "chatgpt_work" ? longPress : undefined} />)}<UpdateBadge status={update} /></div></main>;
}
function ProviderSection({ quota, activity, onExpired, longPress }: { quota: ProviderQuota; activity: AgentActivity; onExpired: (provider: ProviderId, resetAt: string) => void; longPress?: React.HTMLAttributes<HTMLElement> }) { const antigravity = quota.provider === "antigravity"; return <section className={`provider provider--${quota.provider}`} {...longPress} title={longPress ? "길게 눌러 Codex 리셋권 보기" : undefined}><header className="provider__heading" data-tauri-drag-region><span className={`activity-dot ${activity === "active" ? "activity-dot--active" : ""} ${activity === "unknown" ? "activity-dot--unknown" : ""}`} /><h1 data-tauri-drag-region>{providerNames[quota.provider]}</h1></header>{quota.pools.map((pool, index) => <div className="pool" key={`${pool.name ?? "default"}-${index}`}>{antigravity && <div className="pool__name">{pool.name ?? "—"}</div>}<QuotaRow label="5h" window={pool.fiveHour} provider={quota.provider} onExpired={onExpired} /><QuotaRow label="Week" window={pool.weekly} provider={quota.provider} onExpired={onExpired} /></div>)}</section>; }
/**
 * Shown only once a newer release is confirmed. It is absolutely positioned in
 * the surface's free top-right corner so the overlay's fixed height keeps
 * fitting every quota row; the click opens the release page in the browser.
 */
function UpdateBadge({ status }: { status: UpdateStatus | null }) {
  if (status?.state !== "available" || !status.latestVersion) return null;
  return <button type="button" className="update-badge" title={`새 버전 v${status.latestVersion} · 현재 v${status.currentVersion}`} onClick={() => void openReleasePage().catch(() => undefined)}>v{status.latestVersion}</button>;
}
