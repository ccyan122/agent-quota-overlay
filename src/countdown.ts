import type { QuotaWindow } from "./types";

export const formatCountdown = (resetAt: string | null, now = Date.now()): string => {
  if (!resetAt) return "—";
  const resetMillis = Date.parse(resetAt);
  if (!Number.isFinite(resetMillis)) return "—";
  const seconds = Math.max(0, Math.floor((resetMillis - now) / 1000));
  if (seconds >= 86_400) return `${Math.floor(seconds / 86_400)}일`;
  if (seconds >= 3_600) return `${Math.floor(seconds / 3_600)}시간`;
  if (seconds >= 60) return `${Math.floor(seconds / 60)}분`;
  return `${seconds}초`;
};

/** A stable identity prevents the same expired value from queuing a refresh every tick. */
export const expirationKey = (provider: string, window: QuotaWindow): string | null =>
  window.resetAt ? `${provider}:${window.resetAt}` : null;

export const isExpired = (resetAt: string | null, now = Date.now()): boolean => {
  if (!resetAt) return false;
  const resetMillis = Date.parse(resetAt);
  return Number.isFinite(resetMillis) && resetMillis <= now;
};
