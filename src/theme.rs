use gpui_kit::{Rgba, WindowAppearance};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};

const BUILT_IN: [&str; 4] = [
    include_str!("../assets/themes/wax.json"),
    include_str!("../assets/themes/one/one.json"),
    include_str!("../assets/themes/ayu/ayu.json"),
    include_str!("../assets/themes/gruvbox/gruvbox.json"),
];

pub const DEFAULT_DARK: &str = "Wax Dark";
pub const DEFAULT_LIGHT: &str = "Wax Light";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    System,
    Light,
    Dark,
}

impl Mode {
    pub fn parse(s: &str) -> Mode {
        match s.trim().to_ascii_lowercase().as_str() {
            "light" => Mode::Light,
            "dark" => Mode::Dark,
            _ => Mode::System,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::System => "system",
            Mode::Light => "light",
            Mode::Dark => "dark",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    style: Value,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub bg: Rgba,
    pub sidebar: Rgba,
    pub list_bg: Rgba,
    pub sunken: Rgba,
    pub card: Rgba,
    pub border: Rgba,
    pub border_strong: Rgba,
    pub hover: Rgba,
    pub selected: Rgba,
    pub marked: Rgba,
    pub text: Rgba,
    pub subtle: Rgba,
    pub muted: Rgba,
    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub on_accent: Rgba,
    pub button: Rgba,
    pub button_hover: Rgba,
    pub search_bg: Rgba,
    pub search_border: Rgba,
    pub popover: Rgba,
    pub popover_hover: Rgba,
    pub error: Rgba,
    pub error_bg: Rgba,
    pub error_bg_hover: Rgba,
    pub error_bg_strong: Rgba,
    pub warning: Rgba,
    pub warning_bg: Rgba,
    pub warning_border: Rgba,
}

pub fn hex(s: &str) -> Option<Rgba> {
    let h = s.trim().trim_start_matches('#');
    let n = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(|v| v as f32 / 255.);
    match h.len() {
        6 => Some(Rgba { r: n(0)?, g: n(2)?, b: n(4)?, a: 1. }),
        8 => Some(Rgba { r: n(0)?, g: n(2)?, b: n(4)?, a: n(6)? }),
        3 => {
            let d = |i: usize| u8::from_str_radix(&h[i..i + 1].repeat(2), 16).ok().map(|v| v as f32 / 255.);
            Some(Rgba { r: d(0)?, g: d(1)?, b: d(2)?, a: 1. })
        }
        _ => None,
    }
}

pub fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    Rgba { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: 1. }
}

fn over(top: Rgba, base: Rgba) -> Rgba {
    if top.a >= 1. {
        return top;
    }
    mix(base, Rgba { a: 1., ..top }, top.a)
}

fn luminance(c: Rgba) -> f32 {
    let f = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b)
}

pub fn contrast(a: Rgba, b: Rgba) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

const WHITE: Rgba = Rgba { r: 1., g: 1., b: 1., a: 1. };
const BLACK: Rgba = Rgba { r: 0., g: 0., b: 0., a: 1. };

impl Theme {
    fn get(&self, keys: &[&str]) -> Option<Rgba> {
        keys.iter().find_map(|k| self.style.get(*k).and_then(Value::as_str).and_then(hex))
    }

    pub fn palette(&self) -> Palette {
        let dark = self.dark;
        let (ink, paper) = if dark { (WHITE, BLACK) } else { (BLACK, WHITE) };
        let bg = self.get(&["editor.background", "background"]).map(|c| over(c, paper)).unwrap_or(if dark { mix(BLACK, WHITE, 0.1) } else { WHITE });
        let on = |c: Option<Rgba>, base: Rgba, fallback: Rgba| c.map(|c| over(c, base)).unwrap_or(fallback);
        let sidebar = on(self.get(&["panel.background", "surface.background", "background"]), bg, mix(bg, paper, 0.25));
        let list_bg = on(self.get(&["surface.background", "panel.background"]), bg, sidebar);
        let text = on(self.get(&["text", "editor.foreground"]), bg, mix(bg, ink, 0.9));
        let muted_raw = on(self.get(&["text.muted"]), bg, mix(bg, text, 0.7));
        let placeholder = on(self.get(&["text.placeholder", "text.disabled"]), bg, mix(bg, text, 0.5));
        let accent = on(self.get(&["text.accent", "icon.accent", "link_text.hover", "info"]), bg, hex("#4f8ff7").unwrap());
        let border = on(self.get(&["border.variant", "border"]), bg, mix(bg, text, 0.1));
        let border_strong = on(self.get(&["border", "border.variant"]), bg, mix(bg, text, 0.18));
        let hover = on(self.get(&["ghost_element.hover", "element.hover"]), list_bg, mix(list_bg, text, 0.05));
        let selected = on(self.get(&["ghost_element.selected", "element.selected"]), list_bg, mix(list_bg, accent, 0.18));
        let button = on(self.get(&["element.hover", "element.background"]), bg, mix(bg, text, 0.12));
        let button_hover = on(self.get(&["element.active", "element.selected"]), bg, mix(bg, text, 0.22));
        let popover = on(self.get(&["elevated_surface.background", "surface.background"]), bg, mix(bg, text, 0.05));
        let popover_hover = on(self.get(&["ghost_element.active", "element.hover"]), popover, mix(popover, text, 0.08));
        let error = on(self.get(&["error", "deleted", "terminal.ansi.red"]), bg, hex("#f87171").unwrap());
        let warning = on(self.get(&["warning", "modified", "terminal.ansi.yellow"]), bg, hex("#e5c07b").unwrap());
        let on_accent = if contrast(WHITE, accent) >= 3. { WHITE } else { BLACK };
        let muted = [0., 0.25, 0.5, 0.75, 1.].into_iter().map(|t| mix(placeholder, muted_raw, t)).find(|c| contrast(*c, list_bg) >= 3.).unwrap_or(muted_raw);
        Palette {
            dark,
            bg,
            sidebar,
            list_bg,
            sunken: mix(sidebar, paper, 0.2),
            card: on(self.get(&["editor.subheader.background", "surface.background"]), bg, mix(bg, text, 0.04)),
            border,
            border_strong,
            hover,
            selected,
            marked: mix(list_bg, accent, if dark { 0.1 } else { 0.08 }),
            text,
            subtle: muted_raw,
            muted,
            accent,
            accent_hover: mix(accent, ink, 0.15),
            on_accent,
            button,
            button_hover,
            search_bg: on(self.get(&["element.background", "editor.background"]), bg, mix(bg, text, 0.05)),
            search_border: border_strong,
            popover,
            popover_hover,
            error,
            error_bg: mix(bg, error, if dark { 0.14 } else { 0.1 }),
            error_bg_hover: mix(bg, error, if dark { 0.22 } else { 0.16 }),
            error_bg_strong: mix(bg, error, if dark { 0.32 } else { 0.24 }),
            warning,
            warning_bg: mix(bg, warning, if dark { 0.12 } else { 0.14 }),
            warning_border: mix(bg, warning, 0.5),
        }
    }
}

pub fn parse(text: &str) -> Result<Vec<Theme>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not a theme file: {e}"))?;
    let list = v.get("themes").and_then(Value::as_array).ok_or("no \"themes\" list")?;
    let mut out = Vec::new();
    for t in list {
        let name = t.get("name").and_then(Value::as_str).ok_or("a theme without a name")?;
        let dark = t.get("appearance").and_then(Value::as_str).map(|a| a != "light").unwrap_or(true);
        let style = t.get("style").cloned().ok_or_else(|| format!("{name} has no style"))?;
        out.push(Theme { name: name.to_string(), dark, style });
    }
    Ok(out)
}

pub fn folders() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    vec![crate::files::config_dir().join("themes"), home.join(".config/zed/themes")]
}

pub fn theme_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in folders() {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            out.extend(rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")));
        }
    }
    out.sort();
    out
}

pub fn load_all() -> (Vec<Theme>, Vec<String>) {
    let mut themes: Vec<Theme> = BUILT_IN.iter().flat_map(|t| parse(t).unwrap_or_default()).collect();
    let mut problems = Vec::new();
    for path in theme_files() {
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| parse(&t)) {
            Ok(found) => {
                for t in found {
                    themes.retain(|x| x.name != t.name);
                    themes.push(t);
                }
            }
            Err(e) => problems.push(format!("Theme {}: {e}", path.display())),
        }
    }
    (themes, problems)
}

struct State {
    themes: Vec<Theme>,
    palette: Palette,
    chosen: String,
}

static STATE: LazyLock<RwLock<State>> = LazyLock::new(|| {
    let (themes, _) = load_all();
    let palette = themes.iter().find(|t| t.name == DEFAULT_DARK).map(Theme::palette).expect("built-in theme");
    RwLock::new(State { themes, palette, chosen: DEFAULT_DARK.into() })
});

pub fn pal() -> Palette {
    STATE.read().unwrap().palette
}

pub fn names(dark: bool) -> Vec<String> {
    STATE.read().unwrap().themes.iter().filter(|t| t.dark == dark).map(|t| t.name.clone()).collect()
}

pub fn reload() -> Vec<String> {
    let (themes, problems) = load_all();
    STATE.write().unwrap().themes = themes;
    problems
}

pub fn wanted(mode: Mode, light: &str, dark: &str, appearance: WindowAppearance) -> (String, bool) {
    let system_dark = matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark);
    let use_dark = match mode {
        Mode::Dark => true,
        Mode::Light => false,
        Mode::System => system_dark,
    };
    (if use_dark { dark.to_string() } else { light.to_string() }, use_dark)
}

pub fn apply(mode: Mode, light: &str, dark: &str, appearance: WindowAppearance) -> bool {
    let (name, use_dark) = wanted(mode, light, dark, appearance);
    let mut s = STATE.write().unwrap();
    let fallback = if use_dark { DEFAULT_DARK } else { DEFAULT_LIGHT };
    let Some(theme) = s.themes.iter().find(|t| t.name == name).or_else(|| s.themes.iter().find(|t| t.name == fallback)) else {
        return false;
    };
    let (palette, picked) = (theme.palette(), theme.name.clone());
    if s.palette == palette && s.chosen == picked {
        return false;
    }
    s.palette = palette;
    s.chosen = picked;
    true
}

pub fn sync_components(cx: &mut gpui_kit::App) {
    use gpui_kit::component::{Theme as Kit, ThemeMode};
    let p = pal();
    Kit::change(if p.dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx);
    Kit::update(cx, |t| {
        let c = &mut t.colors;
        c.background = p.bg.into();
        c.foreground = p.text.into();
        c.border = p.border_strong.into();
        c.input = p.search_border.into();
        c.ring = p.accent.into();
        c.caret = p.text.into();
        c.selection = Rgba { a: 0.35, ..p.accent }.into();
        c.primary = p.accent.into();
        c.primary_hover = p.accent_hover.into();
        c.primary_foreground = p.on_accent.into();
        c.muted = p.card.into();
        c.muted_foreground = p.muted.into();
        c.popover = p.popover.into();
        c.popover_foreground = p.text.into();
        c.accent = p.popover_hover.into();
        c.accent_foreground = p.text.into();
        c.list_hover = p.hover.into();
        c.list_active = p.selected.into();
        c.secondary = p.button.into();
        c.secondary_hover = p.button_hover.into();
        c.secondary_foreground = p.text.into();
        c.link = p.accent.into();
        c.danger = p.error.into();
        c.warning = p.warning.into();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_themes_load_and_read_well() {
        let themes: Vec<Theme> = BUILT_IN.iter().flat_map(|t| parse(t).unwrap()).collect();
        for want in ["Wax Dark", "Wax Light", "One Dark", "One Light", "Ayu Dark", "Gruvbox Light"] {
            assert!(themes.iter().any(|t| t.name == want), "{want} missing");
        }
        for t in &themes {
            let p = t.palette();
            assert_eq!(p.dark, t.dark);
            assert!(contrast(p.text, p.bg) >= 4.5, "{}: text on background {}", t.name, contrast(p.text, p.bg));
            assert!(contrast(p.muted, p.list_bg) >= 2.5, "{}: muted text {}", t.name, contrast(p.muted, p.list_bg));
            assert!(contrast(p.on_accent, p.accent) >= 2.5, "{}: text on accent {}", t.name, contrast(p.on_accent, p.accent));
            assert!(p.bg.a == 1. && p.selected.a == 1. && p.hover.a == 1.);
        }
    }

    #[test]
    fn wax_dark_keeps_the_original_colours() {
        let themes: Vec<Theme> = BUILT_IN.iter().flat_map(|t| parse(t).unwrap()).collect();
        let p = themes.iter().find(|t| t.name == DEFAULT_DARK).unwrap().palette();
        assert_eq!(p.bg, hex("#1b1b1f").unwrap());
        assert_eq!(p.sidebar, hex("#151518").unwrap());
        assert_eq!(p.list_bg, hex("#18181c").unwrap());
        assert_eq!(p.text, hex("#e8e8ec").unwrap());
        assert_eq!(p.muted, hex("#82828d").unwrap());
        assert_eq!(p.accent, hex("#4f8ff7").unwrap());
        assert_eq!(p.selected, hex("#1f2a40").unwrap());
        assert_eq!(p.on_accent, WHITE);
    }

    #[test]
    fn parses_zed_files_and_fills_gaps() {
        let text = r##"{"name":"Tiny","themes":[{"name":"Tiny Light","appearance":"light","style":{"text":"#111111","editor.background":"#ffffffff","text.accent":"#0066cc"}}]}"##;
        let t = &parse(text).unwrap()[0];
        assert!(!t.dark);
        let p = t.palette();
        assert_eq!(p.text, hex("#111111").unwrap());
        assert!(contrast(p.muted, p.bg) > 1.5 && p.muted != p.text);
        assert!(parse("{}").is_err());
        assert_eq!(hex("#abc"), hex("#aabbcc"));
    }
}
