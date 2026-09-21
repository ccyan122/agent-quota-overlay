import { memo, useEffect, useState } from "react";
import { formatCountdown, isExpired } from "./countdown";
import type { ProviderId, QuotaWindow } from "./types";
export const QuotaRow = memo(function QuotaRow({ label, window: quotaWindow, provider, onExpired }: { label: string; window: QuotaWindow; provider: ProviderId; onExpired: (provider: ProviderId, resetAt: string) => void }) {
  const percent = quotaWindow.remainingPercent;
  return <div className="quota-row"><span className="quota-row__label">{label}</span><span className="quota-row__track" aria-hidden="true"><span className="quota-row__fill" style={{ width: `${percent ?? 0}%` }} /></span><span className="quota-row__percent">{percent == null ? "—" : `${Math.round(percent)}%`}</span><Countdown resetAt={quotaWindow.resetAt} provider={provider} onExpired={onExpired} /></div>;
});
const Countdown = memo(function Countdown({ resetAt, provider, onExpired }: { resetAt: string | null; provider: ProviderId; onExpired: (provider: ProviderId, resetAt: string) => void }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => { const interval = window.setInterval(() => setNow(Date.now()), 1_000); return () => window.clearInterval(interval); }, []);
  useEffect(() => { if (resetAt && isExpired(resetAt, now)) onExpired(provider, resetAt); }, [now, onExpired, provider, resetAt]);
  return <time className="quota-row__reset">{formatCountdown(resetAt, now)}</time>;
});
