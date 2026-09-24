//! Where a window opens: the geometry the main window and the detached chat window
//! remember, checked against the displays connected now.
//!
//! A remembered geometry can outlive its display (an external screen unplugged, a
//! laptop lid closed) or no longer fit it (a smaller resolution). The rule shared by
//! every remembering window: the title bar has to land on some display's visible area
//! (`grabbable_on_some_monitor`, the browser window's check) or the memory is dropped
//! for a centred default; a geometry that passes is then shrunk to that display's
//! visible area and pulled inside it, so no edge is out of reach.
//!
//! The main window remembers one geometry per display: moving it to an external screen
//! does not overwrite where it sat on the built-in one, so unplugging the screen brings
//! back the laptop's own size rather than a big window squeezed down.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::agent_window::WindowGeom;

/// A display's visible area (the work area: the menu bar, Dock and taskbar already
/// removed), in logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

fn area_of(m: &tauri::Monitor) -> Area {
    let scale = m.scale_factor();
    let pos = m.work_area().position.to_logical::<f64>(scale);
    let size = m.work_area().size.to_logical::<f64>(scale);
    Area { x: pos.x, y: pos.y, w: size.width, h: size.height }
}

/// A display's identity across launches: its name plus its native resolution. The
/// position in the arrangement is left out on purpose — rearranging the screens in the
/// system settings is not a different display.
fn display_key(m: &tauri::Monitor) -> String {
    let size = m.size();
    format!("{}|{}x{}", m.name().map(String::as_str).unwrap_or(""), size.width, size.height)
}

/// The connected displays as (identity, visible area); None when they cannot be read.
fn displays(app: &AppHandle) -> Option<Vec<(String, Area)>> {
    let monitors = app.available_monitors().ok()?;
    if monitors.is_empty() {
        return None;
    }
    Some(monitors.iter().map(|m| (display_key(m), area_of(m))).collect())
}

/// The point that has to stay reachable: the middle of the title bar, 20 logical px
/// into the window (the same point `grabbable_on_some_monitor` tests).
fn title_point(g: &WindowGeom) -> (f64, f64) {
    (g.x + g.w / 2.0, g.y + 20.0)
}

fn holds(a: &Area, (px, py): (f64, f64)) -> bool {
    px >= a.x && px <= a.x + a.w && py >= a.y && py <= a.y + a.h
}

/// Shrink a geometry to an area and pull it inside: a window larger than the display
/// takes the whole visible area, and one hanging over an edge is moved back in.
pub fn fit(g: WindowGeom, a: Area) -> WindowGeom {
    let w = g.w.min(a.w);
    let h = g.h.min(a.h);
    let x = g.x.clamp(a.x, a.x + a.w - w);
    let y = g.y.clamp(a.y, a.y + a.h - h);
    WindowGeom { x, y, w, h }
}

/// A window of the given size centred in an area (shrunk to it first), on whole pixels.
pub fn centred(w: f64, h: f64, a: Area) -> WindowGeom {
    let (w, h) = (w.min(a.w).round(), h.min(a.h).round());
    WindowGeom { x: (a.x + (a.w - w) / 2.0).round(), y: (a.y + (a.h - h) / 2.0).round(), w, h }
}

/// The main window's first geometry on a display it has never been on: 85% of the
/// visible area, no smaller than 1280×820 (the size the layout's defaults are tuned
/// for), never larger than the visible area, centred.
fn first_main(a: Area) -> WindowGeom {
    centred((a.w * 0.85).max(1280.0), (a.h * 0.85).max(820.0), a)
}

/// Where a remembered geometry may reopen now: None when its title bar lands on no
/// display (the caller centres a default instead); otherwise the geometry fitted into
/// the display holding its title bar. With no display list available the memory is
/// trusted as it is.
pub fn place(app: &AppHandle, g: WindowGeom) -> Option<WindowGeom> {
    if !crate::browser::window::grabbable_on_some_monitor(app, &g) {
        return None;
    }
    let Some(list) = displays(app) else { return Some(g) };
    let point = title_point(&g);
    Some(match list.iter().find(|(_, a)| holds(a, point)) {
        Some((_, a)) => fit(g, *a),
        None => g,
    })
}

/// The visible area of the display a window is on, else the primary display's.
pub fn area_under(app: &AppHandle, win: &tauri::Window) -> Option<Area> {
    let m = win.current_monitor().ok().flatten().or_else(|| app.primary_monitor().ok().flatten())?;
    Some(area_of(&m))
}

// ---- the main window ----

/// One display's remembered main-window geometry. `geom` is the normal (not maximised)
/// geometry; `maximized` is put back on top of it, so un-maximising lands where it was.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayGeom {
    pub display: String,
    #[serde(flatten)]
    pub geom: WindowGeom,
    #[serde(default)]
    pub maximized: bool,
}

/// How many displays are remembered: a handful covers home, office and a projector;
/// the list is ordered by use, so the oldest falls off.
const MAX_DISPLAYS: usize = 6;

/// The main window's latest geometry in this process, written to settings when the
/// window is closed or hidden and when the app exits (the Moved and Resized events
/// arrive at pointer frequency during a drag — too often for a disk write).
static LATEST: Mutex<Option<DisplayGeom>> = Mutex::new(None);

/// Build the main window where it reopens: the geometry remembered for the most
/// recently used display that is connected now, else 85% of the display the pointer is
/// on. The window is created at that geometry rather than created hidden and moved
/// before its first show: a webview whose window was resized or moved to another
/// display while still hidden sometimes never painted (measured: the page loaded, the
/// window stayed blank), and a window born in place never visibly jumps either.
pub fn create_main(app: &AppHandle) -> tauri::Result<()> {
    let (geom, maximized) = main_geometry(app);
    let mut builder = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
        .title("vemry")
        .min_inner_size(764.0, 480.0)
        .maximized(maximized);
    builder = match geom {
        Some(g) => builder.inner_size(g.w, g.h).position(g.x, g.y),
        None => builder.inner_size(1280.0, 820.0).center(),
    };
    let win = builder.build()?;
    if let Some(g) = geom.filter(|_| !maximized) {
        settle_size(&win.as_ref().window(), g);
    }
    Ok(())
}

/// Apply a built window's size once more. The system creates a window on the primary
/// display and caps its size to that screen before moving it: a size remembered on a
/// larger external display comes out clipped to the built-in one (measured: 2560×1540
/// opened as 1470×923). Now that the window is on its own display the size holds; when
/// nothing was clipped this changes nothing.
pub fn settle_size(win: &tauri::Window, g: WindowGeom) {
    let _ = win.set_size(tauri::LogicalSize::new(g.w, g.h));
}

/// The geometry the main window opens with, and whether it is maximised on top of it;
/// None when no display can be read (the window then takes the default size, centred).
fn main_geometry(app: &AppHandle) -> (Option<WindowGeom>, bool) {
    let Some(list) = displays(app) else { return (None, false) };
    let remembered = crate::settings::load(app).main_window;
    let hit = remembered
        .into_iter()
        .filter(|e| list.iter().any(|(k, _)| *k == e.display))
        .find_map(|e| place(app, e.geom).map(|g| (e, g)));
    if let Some((e, g)) = hit {
        let maximized = e.maximized;
        *crate::lock(&LATEST) = Some(DisplayGeom { geom: g, ..e });
        return (Some(g), maximized);
    }
    (area_at_pointer(app).map(first_main), false)
}

/// The visible area of the display the pointer is on (where the reader is looking when
/// they launch the app), else the primary display's.
fn area_at_pointer(app: &AppHandle) -> Option<Area> {
    let primary = app.primary_monitor().ok().flatten();
    let under = app.cursor_position().ok().and_then(|p| {
        // Where the pointer and the monitors share one pixel space, the lookup answers
        // directly. On a Mac with displays of different scales it does not: the pointer
        // comes back as its logical point times the primary display's scale, while each
        // display's position is in its own pixels (measured: pointer x 3767 on a 1x
        // external display at x 1470, beside a 2x built-in one). There both are brought
        // to logical points and compared
        app.monitor_from_point(p.x, p.y).ok().flatten().or_else(|| {
            let scale = primary.as_ref()?.scale_factor();
            let point = (p.x / scale, p.y / scale);
            app.available_monitors().ok()?.into_iter().find(|m| holds(&bounds_of(m), point))
        })
    });
    let m = under.or(primary)?;
    Some(area_of(&m))
}

/// A display's whole bounds in logical points.
fn bounds_of(m: &tauri::Monitor) -> Area {
    let pos = m.position().to_logical::<f64>(m.scale_factor());
    let size = m.size().to_logical::<f64>(m.scale_factor());
    Area { x: pos.x, y: pos.y, w: size.width, h: size.height }
}

/// Note the main window's geometry after a move or a resize (in memory). Minimised,
/// hidden and full-screen states are not geometry to come back to; a maximised window
/// keeps the normal geometry noted before it and only raises the flag.
pub fn record_main(win: &tauri::Window) {
    if win.is_fullscreen().unwrap_or(false) || !win.is_visible().unwrap_or(true) {
        return;
    }
    let Some(m) = win.current_monitor().ok().flatten() else { return };
    let display = display_key(&m);
    let maximized = win.is_maximized().unwrap_or(false);
    let mut latest = crate::lock(&LATEST);
    if maximized {
        match latest.as_mut() {
            Some(d) if d.display == display => d.maximized = true,
            _ => {
                if let Some(geom) = crate::agent_window::read_geom(win) {
                    *latest = Some(DisplayGeom { display, geom, maximized: true });
                }
            }
        }
        return;
    }
    if let Some(geom) = crate::agent_window::read_geom(win) {
        *latest = Some(DisplayGeom { display, geom, maximized: false });
    }
}

/// Write the noted geometry to settings: it becomes the first entry, replacing that
/// display's older one.
pub fn persist_main(app: &AppHandle) {
    let Some(d) = crate::lock(&LATEST).clone() else { return };
    let _ = crate::settings::update(app, |s| remember(&mut s.main_window, d));
}

/// The list edit behind persist_main: returns whether anything changed.
fn remember(list: &mut Vec<DisplayGeom>, d: DisplayGeom) -> bool {
    if list.first() == Some(&d) {
        return false;
    }
    list.retain(|e| e.display != d.display);
    list.insert(0, d);
    list.truncate(MAX_DISPLAYS);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAPTOP: Area = Area { x: 0.0, y: 25.0, w: 1512.0, h: 920.0 };

    fn g(x: f64, y: f64, w: f64, h: f64) -> WindowGeom {
        WindowGeom { x, y, w, h }
    }

    #[test]
    fn fit_shrinks_to_the_area_and_pulls_inside() {
        // Remembered on a 2560 screen: larger than the laptop in both axes
        assert_eq!(fit(g(100.0, 60.0, 2200.0, 1300.0), LAPTOP), g(0.0, 25.0, 1512.0, 920.0));
        // Fits, but hangs over the right and bottom edges
        assert_eq!(fit(g(900.0, 600.0, 800.0, 500.0), LAPTOP), g(712.0, 445.0, 800.0, 500.0));
        // Already inside: untouched
        assert_eq!(fit(g(40.0, 60.0, 800.0, 500.0), LAPTOP), g(40.0, 60.0, 800.0, 500.0));
    }

    #[test]
    fn first_main_takes_85_percent_with_a_floor_and_the_area_as_ceiling() {
        // A 2560×1415 visible area: 85% of both axes
        let big = Area { x: 0.0, y: 25.0, w: 2560.0, h: 1415.0 };
        assert_eq!(first_main(big), g(192.0, 131.0, 2176.0, 1203.0));
        // A laptop: 85% would be under 1280×820, so the floor wins
        assert_eq!(first_main(LAPTOP), g(114.0, 75.0, 1285.0, 820.0));
        // A small screen: the floor is larger than the area, so the area wins
        let small = Area { x: 0.0, y: 0.0, w: 1280.0, h: 760.0 };
        assert_eq!(first_main(small), g(0.0, 0.0, 1280.0, 760.0));
    }

    #[test]
    fn remember_puts_the_display_first_and_replaces_its_old_entry() {
        let d = |k: &str, x: f64| DisplayGeom { display: k.into(), geom: g(x, 0.0, 800.0, 600.0), maximized: false };
        let mut list = vec![d("a", 1.0), d("b", 2.0)];
        assert!(remember(&mut list, d("b", 3.0)));
        assert_eq!(list, vec![d("b", 3.0), d("a", 1.0)]);
        // The same geometry again: nothing to write
        assert!(!remember(&mut list, d("b", 3.0)));
        for i in 0..10 {
            remember(&mut list, d(&format!("x{i}"), 0.0));
        }
        assert_eq!(list.len(), MAX_DISPLAYS);
        assert_eq!(list[0].display, "x9");
    }

    #[test]
    fn display_geom_round_trips_flat() {
        let d = DisplayGeom { display: "Built-in|3024x1964".into(), geom: g(10.0, 30.0, 1280.0, 820.0), maximized: true };
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(json, r#"{"display":"Built-in|3024x1964","x":10.0,"y":30.0,"w":1280.0,"h":820.0,"maximized":true}"#);
        assert_eq!(serde_json::from_str::<DisplayGeom>(&json).unwrap(), d);
        // An entry written without the flag reads as not maximised
        let old: DisplayGeom = serde_json::from_str(r#"{"display":"a","x":0,"y":0,"w":1,"h":1}"#).unwrap();
        assert!(!old.maximized);
    }
}
