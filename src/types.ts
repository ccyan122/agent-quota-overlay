/** These are the whole credential-free IPC contract. */
export type ProviderId = "chatgpt_work" | "claude_code" | "antigravity";
export type AgentActivity = "active" | "idle" | "unknown";

export interface QuotaWindow {
  remainingPercent: number | null;
  resetAt: string | null;
}

export interface QuotaPool {
  name: string | null;
  fiveHour: QuotaWindow;
  weekly: QuotaWindow;
}

/** One Codex reset credit. Its expiry is known only from the dedicated endpoint. */
export interface ResetCredit {
  expiresAt: string | null;
}

export interface ResetCredits {
  availableCount: number;
  /** Still-available credits, soonest expiry first. Empty when only a count was reported. */
  credits: ResetCredit[];
  /** False when the count came from the usage body, which carries no expiries. */
  expiriesKnown: boolean;
}

export interface ProviderQuota {
  provider: ProviderId;
  pools: QuotaPool[];
  /** Codex only, and absent until a response carries it. */
  resetCredits?: ResetCredits | null;
}

export interface ProviderActivity {
  provider: ProviderId;
  state: AgentActivity;
}

export type UpdateCheckState = "unconfigured" | "pending" | "up_to_date" | "available" | "failed";

export interface UpdateStatus {
  state: UpdateCheckState;
  currentVersion: string;
  latestVersion: string | null;
  releaseUrl: string | null;
  checkedAt: string | null;
}
