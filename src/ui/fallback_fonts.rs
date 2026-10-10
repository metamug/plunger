//! System fonts for scripts the bundled ones don't cover (CJK, Indic, Arabic, symbols...).
//! Without them such text shows as empty boxes. A font is loaded only when text that needs
//! it first appears, so startup and memory stay light for everyone else.

use eframe::egui::{self, FontData, FontFamily, FontId};
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Script {
    Symbols,
    RightToLeft,
    Indic,
    Thai,
    Hangul,
    Cjk,
}

fn script_of(c: char) -> Option<Script> {
    match c as u32 {
        0x2190..=0x2BFF => Some(Script::Symbols),
        0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF => Some(Script::RightToLeft),
        0x0900..=0x0DFF => Some(Script::Indic),
        0x0E00..=0x0EFF => Some(Script::Thai),
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => Some(Script::Hangul),
        0x2E80..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FFFF => Some(Script::Cjk),
        _ => None,
    }
}

/// Font files to try for a script, with the face index inside the file.
fn candidates(script: Script) -> &'static [(&'static str, u32)] {
    #[cfg(windows)]
    {
        match script {
            Script::Symbols => &[("C:\\Windows\\Fonts\\seguisym.ttf", 0)],
            Script::RightToLeft => &[("C:\\Windows\\Fonts\\segoeui.ttf", 0)],
            Script::Indic => &[("C:\\Windows\\Fonts\\Nirmala.ttf", 0), ("C:\\Windows\\Fonts\\Nirmala.ttc", 0)],
            Script::Thai => &[("C:\\Windows\\Fonts\\LeelawUI.ttf", 0), ("C:\\Windows\\Fonts\\tahoma.ttf", 0)],
            Script::Hangul => &[("C:\\Windows\\Fonts\\malgun.ttf", 0)],
            Script::Cjk => &[
                ("C:\\Windows\\Fonts\\msyh.ttc", 0),
                ("C:\\Windows\\Fonts\\YuGothM.ttc", 0),
                ("C:\\Windows\\Fonts\\msjh.ttc", 0),
                ("C:\\Windows\\Fonts\\simsun.ttc", 0),
            ],
        }
    }
    #[cfg(target_os = "macos")]
    {
        match script {
            Script::Symbols | Script::RightToLeft | Script::Indic | Script::Thai => {
                &[("/Library/Fonts/Arial Unicode.ttf", 0), ("/System/Library/Fonts/Supplemental/Arial Unicode.ttf", 0)]
            }
            Script::Hangul | Script::Cjk => {
                &[("/System/Library/Fonts/Supplemental/Arial Unicode.ttf", 0), ("/System/Library/Fonts/PingFang.ttc", 0)]
            }
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        match script {
            Script::Symbols => &[("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0)],
            Script::RightToLeft => &[("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0)],
            Script::Indic => &[
                ("/usr/share/fonts/truetype/noto/NotoSansDevanagari-Regular.ttf", 0),
                ("/usr/share/fonts/truetype/lohit-devanagari/Lohit-Devanagari.ttf", 0),
            ],
            Script::Thai => &[("/usr/share/fonts/truetype/tlwg/Loma.ttf", 0)],
            Script::Hangul | Script::Cjk => &[
                ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
                ("/usr/share/fonts/truetype/wqy/wqy-microhei.ttc", 0),
            ],
        }
    }
}

/// Text past this length isn't scanned: whatever is on screen shows up in the start of it.
const SCAN_LIMIT: usize = 64 * 1024;

/// Loads a system font for any script in `text` that can't be drawn yet. Cheap when there
/// is nothing to do, and each script is tried at most once.
pub fn ensure(ctx: &egui::Context, text: &str) {
    if text.is_ascii() {
        return;
    }
    let tried_id = egui::Id::new("fallback-fonts-tried");
    let mut tried: HashSet<Script> = ctx.data(|d| d.get_temp(tried_id)).unwrap_or_default();

    let mut end = text.len().min(SCAN_LIMIT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut needed = HashSet::new();
    ctx.fonts_mut(|fonts| {
        let font = FontId::proportional(14.0);
        for c in text[..end].chars().filter(|c| !c.is_control()) {
            if let Some(script) = script_of(c) {
                if !tried.contains(&script) && !needed.contains(&script) && !fonts.has_glyph(&font, c) {
                    needed.insert(script);
                }
            }
        }
    });
    if needed.is_empty() {
        return;
    }

    let mut definitions = ctx.fonts(|f| f.definitions().clone());
    let mut added = false;
    for script in needed {
        tried.insert(script);
        for (path, index) in candidates(script) {
            let Ok(bytes) = std::fs::read(path) else { continue };
            let name = format!("fallback-{script:?}");
            let mut data = FontData::from_owned(bytes);
            data.index = *index;
            definitions.font_data.insert(name.clone(), std::sync::Arc::new(data));
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                definitions.families.entry(family).or_default().push(name.clone());
            }
            added = true;
            break;
        }
    }
    ctx.data_mut(|d| d.insert_temp(tried_id, tried));
    if added {
        ctx.set_fonts(definitions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_told_apart() {
        assert_eq!(script_of('日'), Some(Script::Cjk));
        assert_eq!(script_of('한'), Some(Script::Hangul));
        assert_eq!(script_of('न'), Some(Script::Indic));
        assert_eq!(script_of('ع'), Some(Script::RightToLeft));
        assert_eq!(script_of('✓'), Some(Script::Symbols));
        assert_eq!(script_of('é'), None);
        assert_eq!(script_of('a'), None);
    }

    #[test]
    fn plain_text_costs_nothing_and_loads_nothing() {
        let ctx = egui::Context::default();
        crate::test_support::pass(&ctx, Default::default(), |ui| ensure(ui.ctx(), "just ascii, and café"));
        assert!(ctx.data(|d| d.get_temp::<HashSet<Script>>(egui::Id::new("fallback-fonts-tried"))).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn cjk_text_gets_a_system_font_once_it_appears() {
        let ctx = egui::Context::default();
        let font = FontId::proportional(14.0);
        crate::test_support::pass(&ctx, Default::default(), |ui| ensure(ui.ctx(), "日本語"));
        // The new font takes effect on the next frame.
        crate::test_support::pass(&ctx, Default::default(), |_| {});
        if candidates(Script::Cjk).iter().any(|(p, _)| std::path::Path::new(p).exists()) {
            assert!(ctx.fonts_mut(|f| f.has_glyph(&font, '日')));
        }
    }
}
