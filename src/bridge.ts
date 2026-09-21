import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ProviderActivity, ProviderId, ProviderQuota, UpdateStatus } from "./types";

export const QUOTA_UPDATED = "quota://updated";
export const ACTIVITY_UPDATED = "activity://updated";
export const OPACITY_CHANGED = "overlay://opacity";
export const UPDATE_STATUS = "update://status";

export const OVERLAY_WINDOW = "overlay";
export const PANEL_WINDOW = "resets";

export const getQuotaSnapshot = () => invoke<ProviderQuota[]>("get_quota_snapshot");
export const getActivitySnapshot = () => invoke<ProviderActivity[]>("get_activity_snapshot");
export const getOverlaySettings = () => invoke<{ opacity: number }>("get_overlay_settings");
export const refreshQuotas = (manual = false, provider?: ProviderId) => invoke<ProviderQuota[]>("refresh_quotas", { manual, provider });
export const subscribeQuota = (handler: (quotas: ProviderQuota[]) => void): Promise<UnlistenFn> =>
  listen<ProviderQuota[]>(QUOTA_UPDATED, ({ payload }) => handler(payload));
export const subscribeActivity = (handler: (activities: ProviderActivity[]) => void): Promise<UnlistenFn> =>
  listen<ProviderActivity[]>(ACTIVITY_UPDATED, ({ payload }) => handler(payload));

/** Returns whether the reset-credit panel is open after the toggle. */
export const toggleResetPanel = () => invoke<boolean>("toggle_reset_panel");
export const hideResetPanel = () => invoke<void>("hide_reset_panel");

export const getUpdateStatus = () => invoke<UpdateStatus>("get_update_status");
export const openReleasePage = () => invoke<void>("open_release_page");
export const subscribeUpdateStatus = (handler: (status: UpdateStatus) => void): Promise<UnlistenFn> =>
  listen<UpdateStatus>(UPDATE_STATUS, ({ payload }) => handler(payload));
