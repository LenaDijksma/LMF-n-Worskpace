//! Colour themes: Light, Dark and High contrast.
//!
//! Palettes are plain data so they can be tested. The tests below enforce WCAG contrast ratios
//! (4.5:1 for text and meaningful accents; 7:1 in the High contrast theme), so a palette change
//! that makes something unreadable fails `cargo test`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

const fn rgb(hex: u32) -> Rgb {
    Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeKind {
    Light,
    Dark,
    HighContrast,
}

impl ThemeKind {
    pub fn next(self) -> ThemeKind {
        match self {
            ThemeKind::Light => ThemeKind::Dark,
            ThemeKind::Dark => ThemeKind::HighContrast,
            ThemeKind::HighContrast => ThemeKind::Light,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ThemeKind::Light => "Light",
            ThemeKind::Dark => "Dark",
            ThemeKind::HighContrast => "High contrast",
        }
    }

    /// Stable identifier used in the settings file.
    pub fn id(self) -> &'static str {
        match self {
            ThemeKind::Light => "light",
            ThemeKind::Dark => "dark",
            ThemeKind::HighContrast => "high-contrast",
        }
    }

    pub fn from_id(id: &str) -> Option<ThemeKind> {
        match id.trim() {
            "light" => Some(ThemeKind::Light),
            "dark" => Some(ThemeKind::Dark),
            "high-contrast" => Some(ThemeKind::HighContrast),
            _ => None,
        }
    }

    /// Whether standard widgets (buttons, text editor) should use their dark appearance.
    pub fn is_dark(self) -> bool {
        !matches!(self, ThemeKind::Light)
    }

    pub fn colors(self) -> &'static Colors {
        match self {
            ThemeKind::Light => &LIGHT,
            ThemeKind::Dark => &DARK,
            ThemeKind::HighContrast => &HIGH_CONTRAST,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct Colors {
    pub window_bg: Rgb,
    pub sidebar_bg: Rgb,
    pub sidebar_text: Rgb,
    pub sidebar_muted: Rgb,
    pub hover: Rgb,
    pub panel_bg: Rgb,
    pub text: Rgb,
    pub link: Rgb,
    pub diag_bg: Rgb,
    /// Lines and borders (tables, rules); at least 3:1 against the panel (WCAG non-text contrast).
    pub border: Rgb,
    pub error: Rgb,
    pub warning: Rgb,
    pub info: Rgb,
    pub ok: Rgb,
    pub code_fill: Rgb,
    pub code_accent: Rgb,
    pub meta_fill: Rgb,
    pub meta_accent: Rgb,
    pub details_fill: Rgb,
    pub details_accent: Rgb,
    pub tab_fill: Rgb,
    pub tab_accent: Rgb,
    pub unknown_fill: Rgb,
    pub unknown_accent: Rgb,
    /// `(kind, fill, accent)` for each callout kind.
    pub callouts: [(&'static str, Rgb, Rgb); 8],
}

impl Colors {
    /// Fill and accent for a callout kind; unknown kinds use `note`.
    pub fn callout(&self, kind: &str) -> (Rgb, Rgb) {
        self.callouts
            .iter()
            .find(|(k, _, _)| *k == kind)
            .map(|(_, f, a)| (*f, *a))
            .unwrap_or((self.callouts[0].1, self.callouts[0].2))
    }
}

const LIGHT: Colors = Colors {
    window_bg: rgb(0xf5f5f5),
    sidebar_bg: rgb(0xf4f4f6),
    sidebar_text: rgb(0x1f1f1f),
    sidebar_muted: rgb(0x595959),
    hover: rgb(0xe3e6f6),
    panel_bg: rgb(0xffffff),
    text: rgb(0x1f1f1f),
    link: rgb(0x1565c0),
    diag_bg: rgb(0xfafafa),
    border: rgb(0x8a8a96),
    error: rgb(0xb71c1c),
    warning: rgb(0xa34100),
    info: rgb(0x37474f),
    ok: rgb(0x2e7d32),
    code_fill: rgb(0xf1f1f1),
    code_accent: rgb(0x616161),
    meta_fill: rgb(0xeef0ff),
    meta_accent: rgb(0x3949ab),
    details_fill: rgb(0xf5f5f5),
    details_accent: rgb(0x616161),
    tab_fill: rgb(0xeef5ff),
    tab_accent: rgb(0x1565c0),
    unknown_fill: rgb(0xfff8e1),
    unknown_accent: rgb(0x8a5200),
    callouts: [
        ("note", rgb(0xe3f2fd), rgb(0x0d5aa7)),
        ("tip", rgb(0xe8f5e9), rgb(0x2e7d32)),
        ("info", rgb(0xe0f7fa), rgb(0x006064)),
        ("success", rgb(0xe8f5e9), rgb(0x2e7d32)),
        ("warning", rgb(0xfff3e0), rgb(0xa34100)),
        ("danger", rgb(0xffebee), rgb(0xb71c1c)),
        ("question", rgb(0xf3e5f5), rgb(0x7b1fa2)),
        ("example", rgb(0xeceff1), rgb(0x455a64)),
    ],
};

const DARK: Colors = Colors {
    window_bg: rgb(0x1e1e1e),
    sidebar_bg: rgb(0x252526),
    sidebar_text: rgb(0xe6e6e6),
    sidebar_muted: rgb(0xa6a6a6),
    hover: rgb(0x37373d),
    panel_bg: rgb(0x1e1e1e),
    text: rgb(0xe6e6e6),
    link: rgb(0x7ab8ff),
    diag_bg: rgb(0x1b1b1b),
    border: rgb(0x7a7a84),
    error: rgb(0xff8a80),
    warning: rgb(0xffb74d),
    info: rgb(0xb0bec5),
    ok: rgb(0x81c784),
    code_fill: rgb(0x2b2b2b),
    code_accent: rgb(0xb0b0b0),
    meta_fill: rgb(0x262a40),
    meta_accent: rgb(0x9fa8da),
    details_fill: rgb(0x2a2a2a),
    details_accent: rgb(0xb0b0b0),
    tab_fill: rgb(0x212f42),
    tab_accent: rgb(0x7ab8ff),
    unknown_fill: rgb(0x3a3220),
    unknown_accent: rgb(0xffca6b),
    callouts: [
        ("note", rgb(0x1d2d44), rgb(0x7ab8ff)),
        ("tip", rgb(0x1f3524), rgb(0x81c784)),
        ("info", rgb(0x173a3f), rgb(0x4dd0e1)),
        ("success", rgb(0x1f3524), rgb(0x81c784)),
        ("warning", rgb(0x3d2e17), rgb(0xffb74d)),
        ("danger", rgb(0x40201f), rgb(0xff8a80)),
        ("question", rgb(0x35243d), rgb(0xce93d8)),
        ("example", rgb(0x2b3338), rgb(0xb0bec5)),
    ],
};

const HIGH_CONTRAST: Colors = Colors {
    window_bg: rgb(0x000000),
    sidebar_bg: rgb(0x000000),
    sidebar_text: rgb(0xffffff),
    sidebar_muted: rgb(0xe0e0e0),
    hover: rgb(0x333333),
    panel_bg: rgb(0x000000),
    text: rgb(0xffffff),
    link: rgb(0x66d9ff),
    diag_bg: rgb(0x000000),
    border: rgb(0xffffff),
    error: rgb(0xff7b7b),
    warning: rgb(0xffd400),
    info: rgb(0xffffff),
    ok: rgb(0x7dff7d),
    code_fill: rgb(0x111111),
    code_accent: rgb(0xffffff),
    meta_fill: rgb(0x111111),
    meta_accent: rgb(0xffff66),
    details_fill: rgb(0x111111),
    details_accent: rgb(0xffffff),
    tab_fill: rgb(0x111111),
    tab_accent: rgb(0x66d9ff),
    unknown_fill: rgb(0x111111),
    unknown_accent: rgb(0xffd400),
    callouts: [
        ("note", rgb(0x000000), rgb(0x66d9ff)),
        ("tip", rgb(0x000000), rgb(0x7dff7d)),
        ("info", rgb(0x000000), rgb(0x66ffff)),
        ("success", rgb(0x000000), rgb(0x7dff7d)),
        ("warning", rgb(0x000000), rgb(0xffd400)),
        ("danger", rgb(0x000000), rgb(0xff7b7b)),
        ("question", rgb(0x000000), rgb(0xff99ff)),
        ("example", rgb(0x000000), rgb(0xffffff)),
    ],
};

/// WCAG relative luminance.
#[allow(dead_code)]
pub fn luminance(c: Rgb) -> f64 {
    fn lin(v: u8) -> f64 {
        let s = v as f64 / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2)
}

/// WCAG contrast ratio between two colours (1.0 – 21.0).
#[allow(dead_code)]
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [ThemeKind; 3] = [ThemeKind::Light, ThemeKind::Dark, ThemeKind::HighContrast];

    fn need(t: ThemeKind) -> f64 {
        if t == ThemeKind::HighContrast {
            7.0
        } else {
            4.5
        }
    }

    #[test]
    fn text_is_readable_on_every_surface() {
        for t in ALL {
            let c = t.colors();
            let n = need(t);
            for (label, fg, bg) in [
                ("text/panel", c.text, c.panel_bg),
                ("sidebar text/sidebar", c.sidebar_text, c.sidebar_bg),
                ("muted/sidebar", c.sidebar_muted, c.sidebar_bg),
                ("link/panel", c.link, c.panel_bg),
                ("sidebar text/hover", c.sidebar_text, c.hover),
                ("text/code", c.text, c.code_fill),
                ("text/meta", c.text, c.meta_fill),
                ("text/details", c.text, c.details_fill),
                ("text/tab", c.text, c.tab_fill),
                ("text/unknown", c.text, c.unknown_fill),
                ("error/diag", c.error, c.diag_bg),
                ("warning/diag", c.warning, c.diag_bg),
                ("info/diag", c.info, c.diag_bg),
                ("ok/diag", c.ok, c.diag_bg),
                ("code accent", c.code_accent, c.code_fill),
                ("meta accent", c.meta_accent, c.meta_fill),
                ("details accent", c.details_accent, c.details_fill),
                ("tab accent", c.tab_accent, c.tab_fill),
                ("unknown accent", c.unknown_accent, c.unknown_fill),
            ] {
                let r = contrast(fg, bg);
                assert!(r >= n, "{:?} {}: {:.2} < {}", t, label, r, n);
            }
        }
    }

    #[test]
    fn borders_are_visible() {
        for t in ALL {
            let c = t.colors();
            assert!(contrast(c.border, c.panel_bg) >= 3.0, "{:?} border", t);
        }
    }

    #[test]
    fn callouts_are_readable() {
        for t in ALL {
            let c = t.colors();
            for (kind, fill, accent) in c.callouts.iter() {
                assert!(contrast(*accent, *fill) >= need(t), "{:?} {} accent", t, kind);
                assert!(contrast(c.text, *fill) >= need(t), "{:?} {} text", t, kind);
            }
        }
    }

    #[test]
    fn callout_lookup_falls_back_to_note() {
        let c = ThemeKind::Light.colors();
        assert_eq!(c.callout("nonsense"), c.callout("note"));
        assert_ne!(c.callout("danger"), c.callout("note"));
    }

    #[test]
    fn theme_cycle_and_ids() {
        let mut t = ThemeKind::Light;
        for _ in 0..3 {
            assert_eq!(ThemeKind::from_id(t.id()), Some(t));
            t = t.next();
        }
        assert_eq!(t, ThemeKind::Light);
        assert_eq!(ThemeKind::from_id("neon"), None);
        assert!(!ThemeKind::Light.is_dark());
        assert!(ThemeKind::HighContrast.is_dark());
    }

    #[test]
    fn contrast_known_values() {
        assert!((contrast(Rgb(0, 0, 0), Rgb(255, 255, 255)) - 21.0).abs() < 0.01);
        assert!((contrast(Rgb(1, 2, 3), Rgb(1, 2, 3)) - 1.0).abs() < 1e-9);
    }
}
