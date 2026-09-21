import { describe, expect, it } from "vitest";
import { expirationKey, formatCountdown, isExpired } from "./countdown";

const now = Date.UTC(2026, 8, 18, 0, 0, 0);
describe("Korean quota countdown", () => {
  it("keeps only the highest whole time unit", () => {
    expect(formatCountdown("2026-09-23T02:02:01Z", now)).toBe("5일");
    expect(formatCountdown("2026-09-18T03:25:42Z", now)).toBe("3시간");
    expect(formatCountdown("2026-09-18T00:47:51Z", now)).toBe("47분");
    expect(formatCountdown("2026-09-18T00:00:38Z", now)).toBe("38초");
  });
  it("never renders a negative time and uses a timestamp-specific expiry key", () => {
    expect(formatCountdown("2026-09-17T00:00:00Z", now)).toBe("0초");
    expect(isExpired("2026-09-17T00:00:00Z", now)).toBe(true);
    expect(expirationKey("claude_code", { remainingPercent: 12, resetAt: "2026-09-17T00:00:00Z" })).toBe("claude_code:2026-09-17T00:00:00Z");
  });
});
