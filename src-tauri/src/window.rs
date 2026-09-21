//! Native window policy. The webview decides no layout policy; it only requests
//! opacity changes and calls the drag-region API from intentional drag surfaces.

use std::sync::OnceLock;

use tauri::{AppHandle, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Position, Size, WebviewWindow, WindowEvent};
use crate::settings::{SavedPosition, Settings, MIN_OPACITY, MAX_OPACITY};

/// The configured inner size in logical pixels, captured before any display move.
static DESIGN_SIZE: OnceLock<LogicalSize<f64>> = OnceLock::new();

pub fn configure(window: &WebviewWindow, settings: &Settings) -> tauri::Result<()> {
    window.set_always_on_top(true)?;
    window.set_skip_taskbar(true)?;
    let _ = DESIGN_SIZE.set(window.inner_size()?.to_logical(window.scale_factor()?));
    #[cfg(windows)]
    if let (Ok(hwnd), Some(design)) = (window.hwnd(), DESIGN_SIZE.get()) { dpi_guard::install(hwnd.0 as _, *design); }
    restore_position(window, settings)?;
    window.show()?;
    Ok(())
}

/// Saves only physical coordinates plus their source display's scale factor. Physical
/// coordinates avoid the common drift caused by treating logical pixels as global ones.
pub fn remembered_position(window: &WebviewWindow) -> Option<SavedPosition> {
    let position = window.outer_position().ok()?;
    let monitor = window.current_monitor().ok().flatten().or_else(|| window.primary_monitor().ok().flatten());
    Some(SavedPosition {
        x: position.x,
        y: position.y,
        monitor_name: monitor.as_ref().and_then(|item| item.name()).cloned(),
        monitor_scale_factor: monitor.map(|item| item.scale_factor()).unwrap_or(1.0),
    })
}

pub fn restore_position(window: &WebviewWindow, settings: &Settings) -> tauri::Result<()> {
    let Some(saved) = settings.position.as_ref() else { return Ok(()) };
    let monitors = window.available_monitors()?;
    let primary = window.primary_monitor()?;
    let target = monitors.iter().find(|monitor| monitor.name() == saved.monitor_name.as_ref())
        .or_else(|| primary.as_ref())
        .or_else(|| monitors.first());
    let Some(monitor) = target else { return Ok(()) };
    let scale = monitor.scale_factor();
    // Scale x/y relative to its original physical monitor origin. This survives a
    // DPI change while the monitor remains present; a removed display falls back to
    // the first visible work area.
    let original_scale = saved.monitor_scale_factor.max(f64::MIN_POSITIVE);
    let work = monitor.work_area();
    let raw_x = scaled_axis(saved.x, work.position.x, scale, original_scale);
    let raw_y = scaled_axis(saved.y, work.position.y, scale, original_scale);
    let size = window.outer_size()?;
    let x = clamp(raw_x, work.position.x, work.position.x + work.size.width as i32 - size.width as i32);
    let y = clamp(raw_y, work.position.y, work.position.y + work.size.height as i32 - size.height as i32);
    window.set_position(Position::Physical(PhysicalPosition::new(x, y)))
}

pub fn clamp_to_visible_work_area(window: &WebviewWindow) -> tauri::Result<()> {
    let position = window.outer_position()?;
    let monitors = window.available_monitors()?;
    let primary = window.primary_monitor()?;
    let Some(monitor) = monitors.iter().find(|monitor| contains(monitor.work_area().position.x, monitor.work_area().position.y, monitor.work_area().size.width, monitor.work_area().size.height, position.x, position.y)).or_else(|| primary.as_ref()).or_else(|| monitors.first()) else { return Ok(()) };
    let work = monitor.work_area();
    let size = window.outer_size()?;
    let x = clamp(position.x, work.position.x, work.position.x + work.size.width as i32 - size.width as i32);
    let y = clamp(position.y, work.position.y, work.position.y + work.size.height as i32 - size.height as i32);
    if x != position.x || y != position.y { window.set_position(Position::Physical(PhysicalPosition::new(x, y)))?; }
    Ok(())
}

/// Logical gap between the overlay and a panel docked beside it.
const PANEL_GAP: f64 = 8.0;

/// Places `panel` on whichever side of `overlay` has more room on the display
/// the overlay currently sits on, then clamps it into that work area.
///
/// `panel_size` is passed in rather than read back from the window: the side
/// choice depends on the size the caller just applied, and a resize is not
/// guaranteed to be observable by the time this runs.
pub fn place_beside(panel: &WebviewWindow, overlay: &WebviewWindow, panel_size: PhysicalSize<u32>) -> tauri::Result<()> {
    let anchor = overlay.outer_position()?;
    let anchor_size = overlay.outer_size()?;
    let monitor = overlay.current_monitor()?.or(overlay.primary_monitor()?).or_else(|| overlay.available_monitors().ok().and_then(|list| list.into_iter().next()));
    let Some(monitor) = monitor else { return Ok(()) };
    let work = monitor.work_area();
    let gap = (PANEL_GAP * monitor.scale_factor()).round() as i32;
    let x = beside_x(anchor.x, anchor_size.width, panel_size.width, work.position.x, work.size.width, gap);
    let y = clamp(anchor.y, work.position.y, work.position.y + work.size.height as i32 - panel_size.height as i32);
    panel.set_position(Position::Physical(PhysicalPosition::new(x, y)))
}

/// The panel's physical x. The side with more space between the overlay and the
/// work-area edge wins, so an overlay parked against one edge opens its panel
/// into the screen rather than off it. A tie opens to the right.
fn beside_x(anchor_x: i32, anchor_width: u32, panel_width: u32, work_x: i32, work_width: u32, gap: i32) -> i32 {
    let anchor_right = anchor_x.saturating_add(anchor_width as i32);
    let work_right = work_x.saturating_add(work_width as i32);
    let space_left = anchor_x.saturating_sub(work_x);
    let space_right = work_right.saturating_sub(anchor_right);
    let x = if space_right >= space_left { anchor_right.saturating_add(gap) } else { anchor_x.saturating_sub(gap).saturating_sub(panel_width as i32) };
    clamp(x, work_x, work_right - panel_width as i32)
}

/// Safety net for the sparse monitor guard: `dpi_guard` keeps the size right during DPI
/// changes, and this re-applies the logical design size if anything else drifts it.
pub fn restore_design_size(window: &WebviewWindow) -> tauri::Result<()> {
    let Some(design) = DESIGN_SIZE.get().copied() else { return Ok(()) };
    let current = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
    if !size_matches(current, design) { window.set_size(Size::Logical(design))?; }
    Ok(())
}

pub fn attach_monitor_guard(app: AppHandle) {
    // Display-change handling is deliberately independent of webview rendering. It
    // also handles the monitor removed while the overlay is running case.
    if let Some(window) = app.get_webview_window("overlay") {
        window.clone().on_window_event(move |event| {
            if matches!(event, WindowEvent::ScaleFactorChanged { .. }) {
                // Size is corrected synchronously by `dpi_guard`; a queued resize here
                // would land after the next DPI flip and reapply a stale size.
                let _ = clamp_to_visible_work_area(&window);
            }
        });
    }
    // Windows does not guarantee a moved event when a monitor disappears. A sparse
    // guard makes a saved overlay reachable after display topology changes without
    // coupling this recovery to rendering or quota refreshes.
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(15));
        if let Some(window) = app.get_webview_window("overlay") {
            let _ = restore_design_size(&window);
            let _ = clamp_to_visible_work_area(&window);
        }
    });
}

/// Dragging across a DPI boundary makes tao resize the window to a size scaled from
/// its stale state. That intermediate size shifts most of the window back onto the
/// other monitor, so Windows sends WM_DPICHANGED again, and the window shrinks and
/// stutters at the boundary. While tao handles WM_DPICHANGED, this subclass rewrites
/// tao's own resize (WM_WINDOWPOSCHANGING) to Windows' suggested position at the design
/// size for the new DPI, so no wrong size is ever applied.
#[cfg(windows)]
mod dpi_guard {
    use std::{cell::Cell, sync::OnceLock};
    use tauri::LogicalSize;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WINDOWPOS, WM_DPICHANGED, WM_WINDOWPOSCHANGING};

    const SUBCLASS_ID: usize = 0x4c51_4f44;
    static DESIGN: OnceLock<LogicalSize<f64>> = OnceLock::new();
    thread_local! { static TARGET: Cell<Option<RECT>> = const { Cell::new(None) }; }

    pub fn install(hwnd: HWND, design: LogicalSize<f64>) {
        let _ = DESIGN.set(design);
        unsafe { SetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, 0); }
    }

    unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
        match message {
            WM_DPICHANGED => {
                let Some(design) = DESIGN.get() else { return DefSubclassProc(hwnd, message, wparam, lparam) };
                let dpi = u32::from((wparam & 0xffff) as u16);
                let (width, height) = super::physical_design_size(*design, dpi);
                let suggested = &*(lparam as *const RECT);
                let target = RECT { left: suggested.left, top: suggested.top, right: suggested.left + width, bottom: suggested.top + height };
                let previous = TARGET.with(|cell| cell.replace(Some(target)));
                let result = DefSubclassProc(hwnd, message, wparam, lparam);
                // tao normally resizes here; if it skipped that, apply the target ourselves.
                let mut actual = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                if GetWindowRect(hwnd, &mut actual) != 0 && !same_rect(&actual, &target) {
                    SetWindowPos(hwnd, std::ptr::null_mut(), target.left, target.top, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                TARGET.with(|cell| cell.set(previous));
                result
            }
            WM_WINDOWPOSCHANGING => {
                if let Some(target) = TARGET.with(Cell::get) {
                    let position = &mut *(lparam as *mut WINDOWPOS);
                    if position.flags & SWP_NOMOVE == 0 { position.x = target.left; position.y = target.top; }
                    if position.flags & SWP_NOSIZE == 0 { position.cx = target.right - target.left; position.cy = target.bottom - target.top; }
                }
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, message, wparam, lparam),
        }
    }

    fn same_rect(a: &RECT, b: &RECT) -> bool { a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom }
}

fn physical_design_size(design: LogicalSize<f64>, dpi: u32) -> (i32, i32) {
    let scale = f64::from(dpi.max(1)) / 96.0;
    ((design.width * scale).round() as i32, (design.height * scale).round() as i32)
}

/// Rounding across scale factors can differ by a pixel; anything larger is drift.
fn size_matches(current: LogicalSize<f64>, design: LogicalSize<f64>) -> bool {
    (current.width - design.width).abs() <= 1.5 && (current.height - design.height).abs() <= 1.5
}

pub fn opacity_after_wheel(current: u8, delta_y: f64) -> u8 {
    // DOM wheel deltaY is positive when scrolling down, which means less opacity.
    let step = if delta_y.is_sign_positive() { -5 } else { 5 };
    (i16::from(current) + step).clamp(i16::from(MIN_OPACITY), i16::from(MAX_OPACITY)) as u8
}

fn clamp(value: i32, minimum: i32, maximum: i32) -> i32 { value.clamp(minimum, maximum.max(minimum)) }
fn scaled_axis(saved: i32, origin: i32, scale: f64, original_scale: f64) -> i32 {
    let delta = i64::from(saved) - i64::from(origin);
    origin.saturating_add((delta as f64 * scale / original_scale).round().clamp(i32::MIN as f64, i32::MAX as f64) as i32)
}
fn contains(x: i32, y: i32, width: u32, height: u32, point_x: i32, point_y: i32) -> bool {
    point_x >= x && point_x < x + width as i32 && point_y >= y && point_y < y + height as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn opacity_wheel_stays_in_accessible_bounds() {
        assert_eq!(opacity_after_wheel(30, 1.0), 30);
        assert_eq!(opacity_after_wheel(100, -1.0), 100);
        assert_eq!(opacity_after_wheel(88, 1.0), 83);
    }
    #[test] fn visible_area_clamp_preserves_a_reachable_corner() {
        assert_eq!(clamp(-20, 0, 100), 0);
        assert_eq!(clamp(180, 0, 100), 100);
    }
    #[test] fn restore_math_handles_negative_origin_and_dpi_change() { assert_eq!(scaled_axis(-1500, -1920, 1.5, 1.0), -1290); assert_eq!(scaled_axis(500, 0, 2.0, 1.0), 1000); }
    #[test] fn design_size_tolerates_rounding_but_catches_frame_shrink() {
        let design = LogicalSize::new(328.0, 332.0);
        assert!(size_matches(LogicalSize::new(328.8, 331.2), design));
        assert!(!size_matches(LogicalSize::new(312.0, 301.0), design));
        assert!(!size_matches(LogicalSize::new(7.2, 7.2), design));
    }
    #[test] fn design_size_scales_with_monitor_dpi() {
        let design = LogicalSize::new(328.0, 332.0);
        assert_eq!(physical_design_size(design, 96), (328, 332));
        assert_eq!(physical_design_size(design, 120), (410, 415));
        assert_eq!(physical_design_size(design, 144), (492, 498));
    }
    /// Overlay 328 wide, panel 252 wide, 8px gap, on a 1920 work area at x=0.
    fn panel_x(anchor_x: i32) -> i32 { beside_x(anchor_x, 328, 252, 0, 1920, 8) }
    #[test] fn the_panel_opens_toward_the_side_with_more_room() {
        // Parked on the left edge: only the right side fits.
        assert_eq!(panel_x(0), 336);
        // Parked on the right edge: it must open left, not off screen.
        assert_eq!(panel_x(1592), 1332);
        // Just left of center still has more room on the right.
        assert_eq!(panel_x(700), 1036);
        // Just right of center flips to the left.
        assert_eq!(panel_x(900), 640);
    }
    #[test] fn a_tie_opens_right_and_a_cramped_side_still_lands_on_screen() {
        assert_eq!(panel_x(796), 1132);
        // Narrower than overlay + gap + panel: clamped inside the work area.
        assert_eq!(beside_x(10, 328, 252, 0, 400, 8), 148);
        assert_eq!(beside_x(-1900, 328, 252, -1920, 1920, 8), -1564);
    }
    #[test] fn removed_monitor_and_extreme_coordinates_fit_work_area() { assert_eq!(clamp(scaled_axis(9000, 0, 1.0, 1.0), 0, 1592), 1592); assert_eq!(clamp(scaled_axis(i32::MIN, 0, 4.0, 0.5), 0, 1592), 0); }
}
