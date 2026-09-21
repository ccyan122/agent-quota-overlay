import { useEffect, useMemo, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getQuotaSnapshot, hideResetPanel, subscribeQuota } from "./bridge";
import { formatCountdown } from "./countdown";
import type { ProviderQuota, ResetCredits } from "./types";

/**
 * Codex reset credits ("리셋권"): each one immediately clears the 5-hour and
 * weekly windows when spent in Codex. This panel only reports them — spending a
 * credit is irreversible, so it stays where the user already confirms it.
 */
export function ResetPanel() {
  const [credits, setCredits] = useState<ResetCredits | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!isTauri()) return;
    let active = true;
    let stop: (() => void) | undefined;
    const take = (quotas: ProviderQuota[]) => {
      if (!active) return;
      setCredits(quotas.find((quota) => quota.provider === "chatgpt_work")?.resetCredits ?? null);
      setLoaded(true);
    };
    void (async () => {
      try {
        const unlisten = await subscribeQuota(take);
        if (active) stop = unlisten; else unlisten();
      } catch {}
      try { take(await getQuotaSnapshot()); } catch { if (active) setLoaded(true); }
    })();
    return () => { active = false; stop?.(); };
  }, []);

  useEffect(() => {
    const interval = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(interval);
  }, []);

  // Escape only reaches a focused panel; the overlay closes it in every case.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => { if (event.key === "Escape") void hideResetPanel(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const rows = useMemo(() => credits?.credits ?? [], [credits]);

  return (
    <main className="panel" aria-label="Codex 리셋권">
      <div className="panel__surface">
        <header className="panel__heading">
          <h1>Codex 리셋권</h1>
          <button type="button" className="panel__close" onClick={() => void hideResetPanel()} aria-label="닫기">×</button>
        </header>
        <p className="panel__count">
          {credits ? <><strong>{credits.availableCount}</strong>개</> : <strong>—</strong>}
        </p>
        <p className="panel__note">{describe(credits, loaded)}</p>
        {rows.length > 0 && (
          <ol className="panel__list">
            {rows.map((credit, index) => (
              <li className="panel__item" key={`${credit.expiresAt ?? "unknown"}-${index}`}>
                <span className="panel__index">{index + 1}</span>
                <span className="panel__expiry">{formatExpiry(credit.expiresAt)}</span>
                <time className="panel__remaining">{formatCountdown(credit.expiresAt, now)}</time>
              </li>
            ))}
          </ol>
        )}
      </div>
    </main>
  );
}

/** Says what the count means, and says plainly when a fact is simply unknown. */
function describe(credits: ResetCredits | null, loaded: boolean): string {
  if (!credits) return loaded ? "Codex가 리셋권 정보를 주지 않았어요" : "불러오는 중";
  if (credits.availableCount === 0) return "쓸 수 있는 리셋권이 없어요";
  if (!credits.expiriesKnown) return "5시간·주간 한도를 즉시 초기화 · 만료 시각은 받지 못했어요";
  if (credits.credits.length === 0) return "5시간·주간 한도를 즉시 초기화 · 만료 시각 없음";
  return "5시간·주간 한도를 즉시 초기화";
}

const expiryFormat = new Intl.DateTimeFormat("ko-KR", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });

function formatExpiry(value: string | null): string {
  if (!value) return "만료 시각 미상";
  const millis = Date.parse(value);
  return Number.isFinite(millis) ? expiryFormat.format(millis) : "만료 시각 미상";
}
