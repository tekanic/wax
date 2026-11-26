use gpui_kit::*;
use std::path::PathBuf;

const MIN_W: f32 = 720.;
const MIN_H: f32 = 480.;
const DEFAULT: (f32, f32) = (1280., 820.);

fn file() -> PathBuf {
    crate::files::data_dir().join("window.txt")
}

pub fn encode(b: &WindowBounds) -> String {
    let (kind, r) = match b {
        WindowBounds::Windowed(r) => ("windowed", r),
        WindowBounds::Maximized(r) => ("maximized", r),
        WindowBounds::Fullscreen(r) => ("windowed", r),
    };
    format!(
        "{kind} {} {} {} {}\n",
        r.origin.x.as_f32().round(),
        r.origin.y.as_f32().round(),
        r.size.width.as_f32().round(),
        r.size.height.as_f32().round()
    )
}

pub fn decode(text: &str) -> Option<WindowBounds> {
    let mut parts = text.split_whitespace();
    let kind = parts.next()?;
    let n: Vec<f32> = parts.map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let [x, y, w, h] = n[..] else { return None };
    if !(w.is_finite() && h.is_finite() && x.is_finite() && y.is_finite()) {
        return None;
    }
    let r = Bounds::new(point(px(x), px(y)), size(px(w.max(MIN_W)), px(h.max(MIN_H))));
    match kind {
        "windowed" => Some(WindowBounds::Windowed(r)),
        "maximized" => Some(WindowBounds::Maximized(r)),
        _ => None,
    }
}

fn rect(b: &WindowBounds) -> Bounds<Pixels> {
    match b {
        WindowBounds::Windowed(r) | WindowBounds::Maximized(r) | WindowBounds::Fullscreen(r) => *r,
    }
}

pub fn visible_on(b: &WindowBounds, displays: &[Bounds<Pixels>]) -> bool {
    let r = rect(b);
    let title = Bounds::new(r.origin, size(r.size.width, px(30.)));
    displays.iter().any(|d| {
        let t = title.intersect(d);
        t.size.width.as_f32() >= 200. && t.size.height.as_f32() >= 20.
    })
}

pub fn restore(cx: &App) -> WindowBounds {
    let displays: Vec<Bounds<Pixels>> = cx.displays().iter().map(|d| d.bounds()).collect();
    std::fs::read_to_string(file())
        .ok()
        .and_then(|t| decode(&t))
        .filter(|b| visible_on(b, &displays))
        .unwrap_or_else(|| WindowBounds::Windowed(Bounds::centered(None, size(px(DEFAULT.0), px(DEFAULT.1)), cx)))
}

pub fn content(b: WindowBounds, viewport: Size<Pixels>) -> WindowBounds {
    let fit = |r: Bounds<Pixels>| Bounds::new(r.origin, viewport);
    match b {
        WindowBounds::Windowed(r) => WindowBounds::Windowed(fit(r)),
        WindowBounds::Maximized(r) => WindowBounds::Maximized(r),
        WindowBounds::Fullscreen(r) => WindowBounds::Windowed(r),
    }
}

pub fn save(b: &WindowBounds) {
    let text = encode(b);
    if std::fs::read_to_string(file()).ok().as_deref() != Some(text.as_str()) {
        std::fs::create_dir_all(crate::files::data_dir()).ok();
        std::fs::write(file(), text).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode, visible_on};
    use gpui_kit::{point, px, size, Bounds, Pixels, WindowBounds};

    fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    #[test]
    fn round_trips_and_clamps() {
        let w = WindowBounds::Windowed(b(51., 247., 1531., 1147.));
        assert_eq!(decode(&encode(&w)), Some(w));
        let m = WindowBounds::Maximized(b(0., 30., 3440., 1410.));
        assert_eq!(decode(&encode(&m)), Some(m));
        assert_eq!(encode(&WindowBounds::Fullscreen(b(0., 0., 800., 600.))), "windowed 0 0 800 600\n");
        assert_eq!(decode("windowed 10 10 300 200"), Some(WindowBounds::Windowed(b(10., 10., 720., 480.))));
        assert_eq!(decode("windowed 1 2 3"), None);
        assert_eq!(decode("sideways 1 2 3 4"), None);
        assert_eq!(decode(""), None);
    }

    #[test]
    fn only_restores_onto_a_display() {
        let displays = [b(0., 0., 3440., 1440.)];
        assert!(visible_on(&WindowBounds::Windowed(b(1080., 325., 1280., 852.)), &displays));
        assert!(!visible_on(&WindowBounds::Windowed(b(4000., 300., 1280., 852.)), &displays), "monitor unplugged");
        assert!(!visible_on(&WindowBounds::Windowed(b(3400., 300., 1280., 852.)), &displays), "barely on screen");
        assert!(visible_on(&WindowBounds::Windowed(b(-2000., 300., 1280., 852.)), &[b(-2560., 0., 2560., 1440.), displays[0]]));
    }
}
