import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Overlay } from "./Overlay";
import { ResetPanel } from "./ResetPanel";
import { PANEL_WINDOW } from "./bridge";
import "./styles.css";

/** Both windows load this bundle; the window label decides which one renders. */
const isPanel = (() => { try { return getCurrentWindow().label === PANEL_WINDOW; } catch { return false; } })();

createRoot(document.getElementById("root")!).render(<StrictMode>{isPanel ? <ResetPanel /> : <Overlay />}</StrictMode>);
