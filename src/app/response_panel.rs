use super::copy_button;
use super::response_search;
use crate::ui::highlight::markup_segments;
use super::tab::Tab;
use crate::domain::model::{Outcome, ResponseTab};
use crate::ui::icons::{self, Icon};
use crate::ui::theme::{self, palette, status_badge};
use eframe::egui;
use egui_json_tree::render::DefaultRender;
use egui_json_tree::{DefaultExpand, JsonTree};
const LARGE_JSON_BYTES: usize = 200 * 1024;
/// A text box lays out every character it holds, every frame it changes; 11 MB of text cost
/// over a gigabyte. Copy and Save always use the whole body.
const TEXT_PREVIEW_BYTES: usize = 256 * 1024;
/// The current search match, drawn stronger than the others.
const CURRENT_MATCH: egui::Color32 = egui::Color32::from_rgb(255, 120, 0);
/// Frames to keep trying to scroll the current match into view: a collapsed tree node needs one to open.
const SCROLL_FRAMES: u8 = 3;

impl Tab {
    pub(super) fn render_response_section(&mut self, ui: &mut egui::Ui) {
        let resp = match &self.outcome {
            Outcome::Empty => {
                if !self.is_loading() {
                    // Centre the plunger and its two lines of text in the space left below the
                    // request, not just across it: the pane is mostly empty before the first send.
                    let gap = ui.spacing().item_spacing.y;
                    let content = super::emboss::HEIGHT
                        + 10.0
                        + ui.text_style_height(&egui::TextStyle::Body)
                        + ui.text_style_height(&egui::TextStyle::Small)
                        + 2.0 * gap;
                    // With little room left (the request editor is tall), skip the picture and keep the words.
                    let roomy = ui.available_height() > content + 70.0;
                    ui.add_space(if roomy { ((ui.available_height() - content) / 2.0).max(16.0) } else { 8.0 });
                    ui.vertical_centered(|ui| {
                        if roomy {
                            super::emboss::plunger(ui);
                            ui.add_space(10.0);
                        }
                        if let Some(meta) = &self.opened_from {
                            // Opened from the history: say how it went; the response itself is not stored.
                            ui.horizontal(|ui| {
                                // Centre the line: the badge, then who sent it and when.
                                ui.add_space(((ui.available_width() - 330.0) / 2.0).max(0.0));
                                if let Some(status) = meta.status {
                                    status_badge(ui, status as u16, "");
                                } else {
                                    ui.label(egui::RichText::new("no response").color(crate::ui::theme::palette().error));
                                }
                                let by = match meta.source {
                                    crate::store::history::Source::Gui => "you",
                                    crate::store::history::Source::Cli => "the CLI",
                                    crate::store::history::Source::Mcp => "an agent (MCP)",
                                };
                                let time = meta.elapsed_ms.map(|ms| format!(" · {ms} ms")).unwrap_or_default();
                                ui.label(egui::RichText::new(format!("Sent by {by} {}{time}", crate::domain::timefmt::full(&meta.created_at))).weak());
                            });
                            ui.label(egui::RichText::new("The response is not stored. Ctrl+Enter sends it again").weak().small());
                        } else {
                            ui.label(egui::RichText::new("Send a request to see the response here").weak());
                            ui.label(egui::RichText::new("Ctrl+Enter sends from anywhere").weak().small());
                        }
                    });
                }
                return;
            }
            Outcome::Failed(err) => {
                ui.colored_label(palette().error, format!("Request failed: {err}"));
                return;
            }
            Outcome::Response(resp) => resp,
        };

        // Once per response: load a system font if the body has a script the bundled ones lack.
        let scan_id = egui::Id::new(("font-scan", self.id));
        let signature = (resp.size_bytes, resp.status);
        if ui.ctx().data(|d| d.get_temp::<(usize, u16)>(scan_id)) != Some(signature) {
            crate::ui::fallback_fonts::ensure(ui.ctx(), &resp.body);
            ui.ctx().data_mut(|d| d.insert_temp(scan_id, signature));
        }

        ui.add_space(2.0);
        // One row: the Body / Headers tabs, then the status, time and size, then the actions.
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.response_tab, ResponseTab::Body, "Body");
            let headers_label = if resp.headers.is_empty() { "Headers".to_string() } else { format!("Headers ({})", resp.headers.len()) };
            ui.selectable_value(&mut self.response_tab, ResponseTab::Headers, headers_label);
            ui.add_space(8.0);
            status_badge(ui, resp.status, &resp.status_text);
            let clock = crate::domain::timefmt::clock(&resp.sent_at);
            let summary = if clock.is_empty() {
                format!("{} ms · {}", resp.elapsed_ms, format_bytes(resp.size_bytes))
            } else {
                format!("{clock} · {} ms · {}", resp.elapsed_ms, format_bytes(resp.size_bytes))
            };
            ui.label(egui::RichText::new(summary).weak()).on_hover_ui(|ui| timing_details(ui, resp));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let expanded = self.pane == super::tab::Pane::ResponseExpanded;
                if icons::panel_toggle(
                    ui,
                    expanded,
                    "The response fills the window. Click to show the request again",
                    "Expand the response to the whole window",
                )
                .clicked()
                {
                    self.pane = if expanded { super::tab::Pane::Both } else { super::tab::Pane::ResponseExpanded };
                }
                if icons::button(ui, Icon::Download, "Save response body to a file").clicked() {
                    let name = crate::domain::filename::suggested(&self.state.url, &resp.headers, resp.json_value.is_some(), resp.binary.is_some());
                    if let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() {
                        let bytes = resp.binary.as_deref().unwrap_or(resp.body.as_bytes());
                        self.save_error = std::fs::write(&path, bytes)
                            .err()
                            .map(|e| format!("Could not save file: {e}"));
                    }
                }
                // Copies whatever tab is showing.
                match self.response_tab {
                    ResponseTab::Body if resp.binary.is_some() => {}
                    ResponseTab::Body => {
                        copy_button(ui, &mut self.copied_flash, "body", "Copy response body", &resp.body);
                    }
                    ResponseTab::Headers => {
                        let headers: String = resp.headers.iter().map(|(k, v)| format!("{k}: {v}
")).collect();
                        copy_button(ui, &mut self.copied_flash, "headers", "Copy response headers", &headers);
                    }
                }
            });
        });

        if !resp.redirect_chain.is_empty() {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Redirects").strong());
            for (status, location) in &resp.redirect_chain {
                ui.label(format!("{status} → {location}"));
            }
        }

        if resp.truncated {
            let total = resp.total_size.map(|t| format!(" of {}", format_bytes(t as usize))).unwrap_or_default();
            ui.colored_label(
                palette().amber,
                format!("Response too large: showing the first {}{total}.", format_bytes(resp.size_bytes)),
            );
        }
        if let Some(err) = &self.save_error {
            ui.colored_label(palette().error, err);
        }

        ui.add_space(4.0);

        // What the Body tab shows, worked out once so search, highlighting and scrolling agree.
        let on_body = self.response_tab == ResponseTab::Body;
        let json_shown = resp.json_display.as_ref().or(resp.json_value.as_ref());
        let is_text = resp.binary.is_none() && !resp.body.is_empty();
        // The XML/HTML view is built once per response (and only from the first part of a big body):
        // pretty-printing and colouring on every frame used 1.4 GB and a full core for a 4 MB page.
        let want_markup = on_body && json_shown.is_none() && is_text && is_markup_response(&resp.headers);
        if want_markup {
            let key = (resp.body.as_ptr() as usize, resp.body.len(), resp.elapsed_ms, palette().json[1].to_array());
            if self.markup_cache.as_ref().map(|view| view.key) != Some(key) {
                self.markup_cache = Some(MarkupView::build(&resp.body, key));
            }
        }
        let markup = if want_markup { self.markup_cache.as_ref() } else { None };
        let mut cut = resp.body.len().min(TEXT_PREVIEW_BYTES);
        while !resp.body.is_char_boundary(cut) {
            cut -= 1;
        }
        let preview = &resp.body[..cut];

        let search_on = on_body && self.response_search_open && is_text;
        if !search_on {
            self.response_search_focus = false;
        }
        let searching = search_on && !self.response_search_query.is_empty();
        if searching {
            let key = (resp.body.as_ptr() as usize, resp.body.len(), resp.elapsed_ms, self.response_search_query.clone());
            let query = &self.response_search_query;
            self.response_search_cache.refresh(key, || match (json_shown, &markup) {
                (Some(value), _) => (Vec::new(), response_search::json_matches(value, query)),
                (None, Some(view)) => (response_search::text_matches(&view.text, query), Vec::new()),
                (None, None) => (response_search::text_matches(preview, query), Vec::new()),
            });
        }
        let total = match (searching, json_shown.is_some()) {
            (false, _) => 0,
            (true, true) => self.response_search_cache.json.len(),
            (true, false) => self.response_search_cache.text.len(),
        };

        if search_on {
            if self.response_search_index >= total {
                self.response_search_index = 0;
            }
            let id = egui::Id::new(("response-search", self.id));
            ui.horizontal(|ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.response_search_query)
                        .id(id)
                        .hint_text("Find in response")
                        .desired_width(220.0),
                );
                if std::mem::take(&mut self.response_search_focus) {
                    edit.request_focus();
                    let end = self.response_search_query.chars().count();
                    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(end),
                    )));
                    state.store(ui.ctx(), id);
                }
                let mut step: isize = 0;
                if edit.changed() {
                    self.response_search_index = 0;
                    self.response_search_scroll = SCROLL_FRAMES;
                }
                if edit.lost_focus() {
                    let (enter, escape, shift) =
                        ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape), i.modifiers.shift));
                    if enter {
                        step = if shift { -1 } else { 1 };
                        edit.request_focus();
                    } else if escape {
                        self.response_search_open = false;
                    }
                }
                if ui.button("Prev").clicked() {
                    step = -1;
                }
                if ui.button("Next").clicked() {
                    step = 1;
                }
                if step != 0 && total > 0 {
                    self.response_search_index = (self.response_search_index as isize + step).rem_euclid(total as isize) as usize;
                    self.response_search_scroll = SCROLL_FRAMES;
                }
                let more = if total >= response_search::MAX_MATCHES { "+" } else { "" };
                let at = if total == 0 { 0 } else { self.response_search_index + 1 };
                ui.label(format!("{at}/{total}{more}"));
                if ui.button("Close").clicked() {
                    self.response_search_open = false;
                }
            });
        }

        let current_text = if searching && json_shown.is_none() {
            self.response_search_cache.text.get(self.response_search_index).copied()
        } else {
            None
        };
        let current_json = if searching && json_shown.is_some() {
            self.response_search_cache.json.get(self.response_search_index).cloned()
        } else {
            None
        };
        let match_ranges: &[response_search::Range] = if searching { &self.response_search_cache.text } else { &[] };
        let scroll_wanted = self.response_search_scroll > 0;
        let mut scrolled = false;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match self.response_tab {
                ResponseTab::Body if resp.binary.is_some() => {
                    ui.label(egui::RichText::new(format!("Binary response, {}. Not shown as text: use Save to keep it.", format_bytes(resp.size_bytes))).weak());
                }
                ResponseTab::Body => {
                    if let Some(value) = &resp.json_value {
                        let shown = resp.json_display.as_ref().unwrap_or(value);
                        if resp.json_display.is_some() {
                            ui.colored_label(
                                palette().amber,
                                format!(
                                    "Large response ({} values): the tree shows the start of each long list. Copy or Save keeps everything.",
                                    resp.json_nodes
                                ),
                            );
                        }
                        // Expanding every node of a big document stalls the UI.
                        let expand = if searching {
                            DefaultExpand::SearchResults(&self.response_search_query)
                        } else if resp.body.len() > LARGE_JSON_BYTES {
                            DefaultExpand::ToLevel(1)
                        } else {
                            DefaultExpand::All
                        };
                        let tree = JsonTree::new("response-json-tree", shown)
                            .default_expand(expand)
                            .on_render(|ui, node| {
                                let response = node.render_default(ui);
                                let pointer = node.pointer().to_json_pointer_string();
                                if current_json.as_deref() == Some(pointer.as_str()) {
                                    ui.painter().rect_stroke(response.rect.expand(2.0), 3.0, egui::Stroke::new(1.5_f32, CURRENT_MATCH), egui::StrokeKind::Inside);
                                    if scroll_wanted {
                                        response.scroll_to_me(Some(egui::Align::Center));
                                        scrolled = true;
                                    }
                                }
                                response.context_menu(|ui| {
                                    if ui.button("Copy path").clicked() {
                                        ui.ctx().copy_text(crate::ui::json_view::json_path(value, &pointer));
                                        ui.close();
                                    }
                                    if ui.button("Copy value").clicked() {
                                        let text = match value.pointer(&pointer) {
                                            Some(serde_json::Value::String(s)) => s.clone(),
                                            Some(v) => serde_json::to_string_pretty(v).unwrap_or_default(),
                                            None => String::new(),
                                        };
                                        ui.ctx().copy_text(text);
                                        ui.close();
                                    }
                                });
                            })
                            .show(ui);
                        // The tree remembers each node's open/closed state, so a new response or a
                        // changed query must reset it for the expansion rule to apply again.
                        let expand_key = (
                            resp.body.as_ptr() as usize,
                            resp.body.len(),
                            resp.elapsed_ms,
                            searching.then(|| self.response_search_query.clone()),
                        );
                        if self.response_search_cache.expand_key.as_ref() != Some(&expand_key) {
                            tree.reset_expanded(ui);
                            self.response_search_cache.expand_key = Some(expand_key);
                            ui.ctx().request_repaint();
                        }
                    } else if let Some(view) = markup {
                        let formatted = &view.text;
                        if let Some(total) = view.cut_from {
                            ui.colored_label(
                                palette().amber,
                                format!(
                                    "Showing the first {} of {}. Copy or Save keeps everything.",
                                    format_bytes(TEXT_PREVIEW_BYTES),
                                    format_bytes(total)
                                ),
                            );
                        }
                        let mut text: &str = formatted;
                        let segments = &view.segments;
                        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                            let text = text.as_str();
                            let mut job = layout_job(text, segments, match_ranges, current_text);
                            job.wrap.max_width = wrap_width;
                            ui.ctx().fonts_mut(|fonts| fonts.layout_job(job))
                        };
                        let out = theme::area(&mut text)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .layouter(&mut layouter)
                            .show(ui);
                        if let (true, Some((start, _))) = (scroll_wanted, current_text) {
                            scroll_to_char(ui, &out.galley, out.galley_pos, formatted, start);
                            scrolled = true;
                        }
                    } else if resp.body.is_empty() {
                        ui.label(egui::RichText::new("Empty body").weak());
                    } else {
                        // `&str` is a read-only text buffer: selectable and copyable,
                        // but no per-frame clone of the body and no accidental edits.
                        if cut < resp.body.len() {
                            ui.colored_label(
                                palette().amber,
                                format!(
                                    "Showing the first {} of {}. Copy or Save keeps everything.",
                                    format_bytes(cut),
                                    format_bytes(resp.body.len())
                                ),
                            );
                        }
                        if searching {
                            let segments = [(0, preview.len(), ui.visuals().text_color())];
                            let mut job = layout_job(preview, &segments, match_ranges, current_text);
                            job.wrap.max_width = ui.available_width();
                            let galley = ui.ctx().fonts_mut(|fonts| fonts.layout_job(job));
                            let label = ui.add(egui::Label::new(galley.clone()));
                            if let (true, Some((start, _))) = (scroll_wanted, current_text) {
                                scroll_to_char(ui, &galley, label.rect.min, preview, start);
                                scrolled = true;
                            }
                        } else {
                            let mut text: &str = preview;
                            // JSON that failed to parse, a form, or plain text; big previews stay plain so
                            // building the colours never costs more than drawing the text.
                            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                                let text = text.as_str();
                                let mut job = crate::ui::highlight::body(text);
                                job.wrap.max_width = wrap_width;
                                ui.ctx().fonts_mut(|fonts| fonts.layout_job(job))
                            };
                            ui.add(
                                theme::area(&mut text)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY)
                                    .layouter(&mut layouter),
                            );
                        }
                    }
                }
                ResponseTab::Headers => {
                    egui::Grid::new("response-headers").num_columns(2).spacing([16.0, 6.0]).striped(true).show(
                        ui,
                        |ui| {
                            for (k, v) in &resp.headers {
                                ui.label(egui::RichText::new(k).monospace().weak());
                                ui.add(egui::Label::new(egui::RichText::new(v).monospace()).wrap());
                                ui.end_row();
                            }
                        },
                    );
                }
            });
        self.response_search_scroll = if scrolled { 0 } else { self.response_search_scroll.saturating_sub(1) };
        if self.response_search_scroll > 0 {
            ui.ctx().request_repaint();
        }
    }
}

/// What the time label shows on hover: where the time went and what was sent and received.
fn timing_details(ui: &mut egui::Ui, resp: &crate::domain::model::ResponseData) {
    egui::Grid::new("timing-details").num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
        let row = |ui: &mut egui::Ui, label: &str, value: String| {
            ui.label(egui::RichText::new(label).weak());
            ui.label(value);
            ui.end_row();
        };
        if !resp.sent_at.is_empty() {
            row(ui, "Sent at", crate::domain::timefmt::full(&resp.sent_at));
        }
        row(ui, "Waiting (TTFB)", format!("{} ms", resp.ttfb_ms));
        row(ui, "Download", format!("{} ms", resp.elapsed_ms.saturating_sub(resp.ttfb_ms)));
        row(ui, "Total", format!("{} ms", resp.elapsed_ms));
        row(ui, "Request body", format_request_bytes(resp.request_size_bytes));
        row(ui, "Response body", format_bytes(resp.size_bytes));
    });
}

pub(super) fn format_request_bytes(n: Option<usize>) -> String {
    n.map(format_bytes).unwrap_or_else(|| "?".into())
}

fn is_markup_response(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        if !name.eq_ignore_ascii_case("content-type") { return false; }
        let value = value.to_ascii_lowercase();
        value.contains("application/xml") || value.contains("text/xml") || value.contains("+xml") || value.contains("text/html")
    })
}

/// The pretty-printed, coloured form of an XML or HTML response, built once per response.
pub(super) struct MarkupView {
    /// Which response and theme this was built for: body address and length, elapsed time, a theme colour.
    key: (usize, usize, u128, [u8; 4]),
    text: String,
    segments: Vec<(usize, usize, egui::Color32)>,
    /// The body was longer than the preview limit; this is its full length.
    cut_from: Option<usize>,
}

impl MarkupView {
    fn build(body: &str, key: (usize, usize, u128, [u8; 4])) -> Self {
        let mut cut = body.len().min(TEXT_PREVIEW_BYTES);
        while !body.is_char_boundary(cut) {
            cut -= 1;
        }
        let text = pretty_markup(&body[..cut]);
        let segments = markup_segments(&text);
        Self { key, text, segments, cut_from: (cut < body.len()).then_some(body.len()) }
    }
}

fn pretty_markup(input: &str) -> String {
    let input = input.trim();
    let mut out = String::new();
    let mut depth = 0usize;
    let mut pos = 0usize;
    while let Some(rel) = input[pos..].find('<') {
        let start = pos + rel;
        let text = input[pos..start].trim();
        if !text.is_empty() { out.push_str(&"  ".repeat(depth)); out.push_str(text); out.push('\n'); }
        let Some(end_rel) = input[start..].find('>') else { out.push_str(&input[start..]); return out; };
        let end = start + end_rel + 1;
        let tag = &input[start..end];
        let closing = tag.starts_with("</");
        let name = tag[1..].split(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/').next().unwrap_or_default();
        let html_void = matches!(name.to_ascii_lowercase().as_str(), "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "param" | "source" | "track" | "wbr");
        let standalone = tag.ends_with("/>") || tag.starts_with("<?") || tag.starts_with("<!") || html_void;
        if closing { depth = depth.saturating_sub(1); }
        out.push_str(&"  ".repeat(depth)); out.push_str(tag); out.push('\n');
        if !closing && !standalone { depth += 1; }
        pos = end;
    }
    let tail = input[pos..].trim();
    if !tail.is_empty() { out.push_str(&"  ".repeat(depth)); out.push_str(tail); }
    out.trim_end().to_string()
}

/// Lays `text` out in `segments`' colours, with `matches` (sorted, non-overlapping byte
/// ranges) highlighted and the `current` one stronger.
fn layout_job(
    text: &str,
    segments: &[(usize, usize, egui::Color32)],
    matches: &[response_search::Range],
    current: Option<response_search::Range>,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let font_id = egui::FontId::monospace(13.0);
    let plain = |color| egui::TextFormat { font_id: font_id.clone(), color, ..Default::default() };
    let marked = |is_current: bool| egui::TextFormat {
        font_id: font_id.clone(),
        color: egui::Color32::BLACK,
        background: if is_current { CURRENT_MATCH } else { palette().amber },
        ..Default::default()
    };
    let mut next = 0;
    for &(seg_start, seg_end, color) in segments {
        let mut at = seg_start;
        while at < seg_end {
            while next < matches.len() && matches[next].1 <= at {
                next += 1;
            }
            match matches.get(next) {
                Some(&(start, end)) if start < seg_end => {
                    if start > at {
                        job.append(&text[at..start], 0.0, plain(color));
                        at = start;
                    }
                    let stop = end.min(seg_end);
                    job.append(&text[at..stop], 0.0, marked(current == Some((start, end))));
                    at = stop;
                }
                _ => {
                    job.append(&text[at..seg_end], 0.0, plain(color));
                    at = seg_end;
                }
            }
        }
    }
    job
}

/// Scrolls the character at byte offset `start` of `text` into view, given where `galley` was drawn.
fn scroll_to_char(ui: &egui::Ui, galley: &egui::Galley, origin: egui::Pos2, text: &str, start: usize) {
    let index = text[..start].chars().count();
    let rect = galley.pos_from_cursor(egui::text::CCursor::new(index)).translate(origin.to_vec2());
    ui.scroll_to_rect(rect, Some(egui::Align::Center));
}

pub(super) fn format_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::{format_bytes, format_request_bytes, layout_job, pretty_markup};
    use crate::ui::highlight::markup_segments;
    use crate::app::response_search::text_matches;

    fn rendered(job: &eframe::egui::text::LayoutJob) -> String {
        job.sections.iter().map(|s| &job.text[s.byte_range.start.0..s.byte_range.end.0]).collect()
    }

    #[test]
    fn pretty_markup_keeps_html_void_elements_at_the_current_depth() {
        assert_eq!(pretty_markup("<div><img src=\"x\"><br><span>text</span></div>"), "<div>\n  <img src=\"x\">\n  <br>\n  <span>\n    text\n  </span>\n</div>");
    }

    #[test]
    fn a_big_markup_body_is_formatted_from_its_first_part_only() {
        let body = "<a>x</a>".repeat(100_000);
        let view = super::MarkupView::build(&body, (0, 0, 0, [0; 4]));
        assert_eq!(view.cut_from, Some(body.len()));
        assert!(view.text.len() < super::TEXT_PREVIEW_BYTES * 4, "formatted {} bytes", view.text.len());
        assert!(!view.segments.is_empty());
        // a cut inside a multi-byte character must not panic
        let accents = "<p>é</p>".repeat(200_000);
        assert!(super::MarkupView::build(&accents, (0, 0, 0, [0; 4])).cut_from.is_some());
        // a small body is shown whole
        assert_eq!(super::MarkupView::build("<a>1</a>", (0, 0, 0, [0; 4])).cut_from, None);
    }

    #[test]
    fn markup_segments_cover_the_whole_text_even_with_an_unterminated_tag() {
        for text in ["<a>hi</a>", "plain", "<a>hi <b", "<<>>", ""] {
            let covered: usize = markup_segments(text).iter().map(|&(s, e, _)| e - s).sum();
            assert_eq!(covered, text.len(), "{text:?}");
        }
    }

    #[test]
    fn highlighting_never_changes_the_text_it_draws() {
        let text = "<root>\n  <item id=\"1\">aaa item</item>\n</root>";
        for query in ["a", "aa", "item", "item id", "<", ">", "root", "zzz"] {
            let matches = text_matches(text, query);
            let job = layout_job(text, &markup_segments(text), &matches, matches.first().copied());
            assert_eq!(rendered(&job), text, "query {query:?}");
        }
    }

    #[test]
    fn format_bytes_picks_unit() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(format_request_bytes(Some(1536)), "1.5 KB");
        assert_eq!(format_request_bytes(None), "?");
    }
}
