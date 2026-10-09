//! Tests that draw the whole window headless and drive it like a user would.

use super::*;
use crate::model::{BodyMode, FieldKind, FormField, KeyValue, Outcome, RequestTab, ResponseData, ResponseTab, Variable};
use crate::request::build_request;
use crate::secrets::test_support::MemoryStore;

fn app(state: PersistedState) -> ApiTesterApp {
    ApiTesterApp::new(state, Some(History::in_memory()), Box::new(MemoryStore::default()))
}

/// Runs a few real egui frames (layout and all) without a window.
fn draw(app: &mut ApiTesterApp) {
    let ctx = egui::Context::default();
    theme::apply_theme(&ctx, ThemeChoice::Dark);
    for _ in 0..3 {
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.render_ui(ctx));
    }
}

fn press_escape(app: &mut ApiTesterApp) {
    let ctx = egui::Context::default();
    let _ = ctx.run(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ctx| app.handle_shortcuts(ctx),
    );
}

fn press_tab(app: &mut ApiTesterApp, modifiers: egui::Modifiers) {
    let ctx = egui::Context::default();
    let _ = ctx.run(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        },
        |ctx| app.handle_shortcuts(ctx),
    );
}

#[test]
fn escape_cancels_only_an_in_flight_request() {
    let mut app = app(PersistedState { url: "http://127.0.0.1:1".into(), ..Default::default() });
    app.trigger_send(&egui::Context::default());
    assert!(app.tab().is_loading());

    press_escape(&mut app);
    assert!(!app.tab().is_loading());

    press_escape(&mut app);
    assert!(!app.tab().is_loading());
}

#[test]
fn command_tab_cycles_forward_and_backward_with_wraparound() {
    let mut app = app(PersistedState::default());
    app.new_tab();
    app.new_tab();
    app.activate(0);

    press_tab(&mut app, egui::Modifiers::COMMAND);
    assert_eq!(app.active, 1);
    press_tab(&mut app, egui::Modifiers::COMMAND);
    assert_eq!(app.active, 2);
    press_tab(&mut app, egui::Modifiers::COMMAND);
    assert_eq!(app.active, 0);

    let previous = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
    press_tab(&mut app, previous);
    assert_eq!(app.active, 2);
    press_tab(&mut app, previous);
    assert_eq!(app.active, 1);
}

#[test]
fn command_l_focuses_and_selects_the_url() {
    let mut a = app(PersistedState {
        url: "https://example.com/original".into(),
        ..Default::default()
    });
    let ctx = egui::Context::default();
    theme::apply_theme(&ctx, ThemeChoice::Dark);
    for _ in 0..2 {
        let _ = ctx.run(egui::RawInput::default(), |ctx| a.render_ui(ctx));
    }

    let url_id = egui::Id::new(("url", a.tab().id));
    let _ = ctx.run(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::L,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
            ..Default::default()
        },
        |ctx| {
            a.handle_shortcuts(ctx);
            a.render_ui(ctx);
        },
    );

    assert_eq!(ctx.memory(|memory| memory.focused()), Some(url_id));
    let _ = ctx.run(
        egui::RawInput {
            events: vec![egui::Event::Text("https://example.org".into())],
            ..Default::default()
        },
        |ctx| a.render_ui(ctx),
    );
    assert_eq!(a.tab().state.url, "https://example.org");
}

#[test]
fn typing_a_query_into_the_url_fills_the_params_table() {
    let mut a = app(PersistedState::default());
    a.tab_mut().state.url = "https://h/x?include=events.player&q=a%20b".into();
    draw(&mut a);
    let named: Vec<(&str, &str)> =
        a.tab_mut().state.params.iter().filter(|p| !p.key.is_empty()).map(|p| (p.key.as_str(), p.value.as_str())).collect();
    assert_eq!(named, vec![("include", "events.player"), ("q", "a b")]);
}

#[test]
fn editing_a_param_row_rewrites_the_url_query() {
    let mut a = app(PersistedState { url: "https://h/x?a=1#top".into(), ..Default::default() });
    a.tab_mut().state.params[0].value = "two words".into();
    a.tab_mut().state.params.push(KeyValue { key: "b".into(), value: "&".into(), enabled: true });
    a.tab_mut().render_params_rows_changed_for_test();
    assert_eq!(a.tab_mut().state.url, "https://h/x?a=two%20words&b=%26#top");
    // The URL change came from the table, so the next frame keeps the rows as they are.
    draw(&mut a);
    assert_eq!(a.tab_mut().state.params[1].value, "&");
}

#[test]
fn unticking_a_row_drops_it_from_the_url_but_keeps_it_in_the_table() {
    let mut a = app(PersistedState { url: "https://h/x?a=1&b=2".into(), ..Default::default() });
    a.tab_mut().state.params[0].enabled = false;
    a.tab_mut().render_params_rows_changed_for_test();
    assert_eq!(a.tab_mut().state.url, "https://h/x?b=2");
    draw(&mut a);
    assert_eq!((a.tab().state.params[0].key.as_str(), a.tab().state.params[0].enabled), ("a", false));
}

#[test]
fn old_saved_state_moves_its_params_into_the_url_once() {
    let mut a = app(PersistedState {
        url: "https://h/x".into(),
        params: vec![KeyValue { key: "page".into(), value: "2".into(), enabled: true }],
        ..Default::default()
    });
    draw(&mut a);
    assert_eq!(a.tab_mut().state.url, "https://h/x?page=2");
    let sent = build_request(&a.tab_mut().state, "").unwrap();
    assert_eq!(sent.url, "https://h/x?page=2");
}

fn busy_state() -> PersistedState {
    PersistedState {
        headers_text: "Accept: */*\nX-Trace: 1".into(),
        params: vec![KeyValue { key: "page".into(), value: "2".into(), enabled: true }],
        variables: vec![Variable { name: "host".into(), value: "localhost".into(), secret: false, remember: false }],
        multipart_fields: vec![
            FormField { key: "title".into(), kind: FieldKind::Text, value: "hi".into(), enabled: true },
            FormField { key: "doc".into(), kind: FieldKind::File, value: "C:/x/report.pdf".into(), enabled: true },
        ],
        ..Default::default()
    }
}

#[test]
fn every_request_tab_draws_and_keeps_one_spare_row() {
    let mut a = app(busy_state());
    for tab in [
        RequestTab::Params,
        RequestTab::Auth,
        RequestTab::Headers,
        RequestTab::Body,
        RequestTab::Variables,
        RequestTab::Options,
    ] {
        a.tab_mut().request_tab = tab;
        draw(&mut a);
    }
    assert!(a.tab_mut().state.params.last().unwrap().is_blank());
    assert_eq!(a.tab_mut().state.params.iter().filter(|p| p.is_blank()).count(), 1);
    assert!(a.tab_mut().state.variables.last().unwrap().is_blank());
    assert!(a.tab_mut().header_rows.last().unwrap().0.is_empty());
    assert_eq!(a.tab_mut().state.headers_text, "Accept: */*\nX-Trace: 1", "drawing must not alter the headers");
}

#[test]
fn every_body_mode_draws_and_form_data_keeps_a_spare_row() {
    let mut a = app(busy_state());
    a.tab_mut().request_tab = RequestTab::Body;
    for mode in [BodyMode::None, BodyMode::Json, BodyMode::Multipart, BodyMode::UrlEncoded, BodyMode::Raw] {
        a.tab_mut().state.body_mode = mode;
        draw(&mut a);
    }
    // the last real row is a File, which counts as in use, so one blank follows it
    let fields = &a.tab_mut().state.multipart_fields;
    assert_eq!(fields.len(), 3);
    assert!(fields[2].is_blank() && fields[2].kind == FieldKind::Text);
    assert_eq!(fields[1].value, "C:/x/report.pdf");
}

#[test]
fn invalid_and_variable_json_bodies_draw() {
    let mut a = app(busy_state());
    a.tab_mut().request_tab = RequestTab::Body;
    a.tab_mut().state.body_mode = BodyMode::Json;
    for body in ["{\"a\":", "{\"n\": {{count}}}", "", "   "] {
        a.tab_mut().state.json_body = body.into();
        draw(&mut a);
    }
}

#[test]
fn every_pane_layout_draws_with_and_without_a_response() {
    use crate::app::tab::Pane;
    let mut a = app(busy_state());
    for pane in [Pane::Both, Pane::RequestHidden, Pane::ResponseHidden] {
        for request_tab in [RequestTab::Params, RequestTab::Headers, RequestTab::Body, RequestTab::Variables] {
            a.tab_mut().pane = pane;
            a.tab_mut().request_tab = request_tab;
            a.tab_mut().outcome = Outcome::Empty;
            draw(&mut a);
            a.tab_mut().outcome = Outcome::Response(Box::new(response("{\"a\": 1}", true, false)));
            draw(&mut a);
        }
    }
}

#[test]
fn a_dragged_divider_and_a_history_note_draw_and_double_click_resets() {
    use crate::app::tab::{HistoryMeta, Pane};
    let mut a = app(busy_state());
    a.tab_mut().request_tab = RequestTab::Body;
    a.tab_mut().state.body_mode = BodyMode::Json;
    a.tab_mut().state.json_body = "{\"a\": 1}".into();
    for height in [None, Some(10.0), Some(240.0), Some(5000.0)] {
        a.tab_mut().request_height = height;
        a.tab_mut().outcome = Outcome::Empty;
        draw(&mut a);
        a.tab_mut().opened_from = Some(HistoryMeta { created_at: "2026-10-09T06:11:03Z".into(), status: Some(200), elapsed_ms: Some(7), source: crate::history::Source::Mcp });
        draw(&mut a);
        a.tab_mut().opened_from = None;
        a.tab_mut().outcome = Outcome::Response(Box::new(response("{\"a\": 1}", true, false)));
        draw(&mut a);
    }
    // the editor is given more lines when it is given the room
    a.tab_mut().request_height = Some(500.0);
    draw(&mut a);
    assert!(a.tab().editor_rows > 8, "{} rows", a.tab().editor_rows);
    a.tab_mut().request_height = None;
    a.tab_mut().pane = Pane::Both;
    draw(&mut a);
    assert_eq!(a.tab().editor_rows, 8);
}

#[test]
fn the_agents_dialog_draws_in_both_scopes_without_writing_anything() {
    let mut a = app(PersistedState::default());
    a.open_agents_dialog();
    draw(&mut a);
    let dialog = a.agents_dialog.as_mut().unwrap();
    dialog.scope = crate::install::Scope::Project;
    dialog.plan();
    draw(&mut a);
    assert!(a.agents_dialog.is_some());
}

fn response(body: &str, json: bool, truncated: bool) -> ResponseData {
    ResponseData {
        sent_at: "2026-10-09T06:11:03Z".into(),
        status: 200,
        status_text: "OK".into(),
        ttfb_ms: 1,
        elapsed_ms: 12,
        size_bytes: body.len(),
        request_size_bytes: Some(0),
        headers: vec![("content-type".into(), "application/json".into())],
        redirect_chain: vec![],
        body: body.into(),
        raw_text: None,
        json_value: json.then(|| serde_json::from_str(body).unwrap()),
        truncated,
        total_size: truncated.then_some(99_999_999),
        binary: None,
        json_display: None,
        json_nodes: 0,
    }
}

#[test]
fn every_response_state_draws() {
    let mut a = app(PersistedState::default());
    for outcome in [
        Outcome::Empty,
        Outcome::Failed("boom".into()),
        Outcome::Response(Box::new(response("{\"a\":[1,2,{\"b\":null}]}", true, false))),
        Outcome::Response(Box::new(ResponseData { binary: Some(vec![0, 1, 2, 255]), body: String::new(), size_bytes: 4, json_value: None, ..response("", false, false) })),
        Outcome::Response(Box::new(response("plain text", false, false))),
        Outcome::Response(Box::new(response("{\"a\":1}", true, true))),
    ] {
        a.tab_mut().outcome = outcome;
        for tab in [ResponseTab::Body, ResponseTab::Headers] {
            a.tab_mut().response_tab = tab;
            draw(&mut a);
        }
    }
}

#[test]
fn history_and_import_panels_draw() {
    let mut a = app(PersistedState::default());
    if let Some(h) = &a.history {
        h.insert(&busy_state(), Some(200), Some(5)).unwrap();
        h.insert(&PersistedState { url: "https://exämple.com/ü/🚀/long/long/long/long/long/long".into(), ..Default::default() }, None, None)
            .unwrap();
    }
    a.refresh_lists();
    assert_eq!(a.history_entries.len(), 2);
    a.open_curl_dialog();
    draw(&mut a);
}

#[test]
fn an_undefined_variable_stops_the_send_and_names_it() {
    let mut a = app(PersistedState { url: "http://{{host}}/x/{{id}}".into(), ..Default::default() });
    a.trigger_send(&egui::Context::default());
    assert!(!a.tab().is_loading(), "nothing should be in flight");
    let Outcome::Failed(msg) = &a.tab_mut().outcome else { panic!("expected a failure message") };
    assert!(msg.contains("{{host}}") && msg.contains("{{id}}"), "{msg}");
    assert_eq!(a.tab_mut().state.url, "http://{{host}}/x/{{id}}", "the template must be left untouched");
    assert!(a.history_entries.is_empty(), "a blocked send is not a request");
}

#[test]
fn a_bare_host_gets_its_scheme_in_the_field_but_a_template_does_not() {
    let mut a = app(PersistedState { url: "localhost:3000/x".into(), ..Default::default() });
    a.trigger_send(&egui::Context::default());
    assert_eq!(a.tab_mut().state.url, "http://localhost:3000/x");
    a.tab_mut().cancel();
    assert!(!a.tab().is_loading());

    a.tab_mut().state.url = "{{base}}/x".into();
    a.tab_mut().state.variables = vec![Variable { name: "base".into(), value: "localhost:1".into(), secret: false, remember: false }];
    a.trigger_send(&egui::Context::default());
    assert_eq!(a.tab_mut().state.url, "{{base}}/x");
    a.tab_mut().cancel();
}

#[test]
fn loading_history_keeps_session_variables_and_options() {
    let mut a = app(PersistedState {
        variables: vec![Variable { name: "keep".into(), value: "me".into(), secret: false, remember: false }],
        insecure_tls: true,
        ..Default::default()
    });
    let entry = {
        let h = a.history.as_ref().unwrap();
        h.insert(&PersistedState { url: "http://old/x".into(), ..Default::default() }, Some(200), Some(1)).unwrap();
        h.search_recent("", 1).unwrap().remove(0)
    };
    a.new_tab(); // not pristine any more once it has a URL
    a.tab_mut().state.url = "http://current".into();
    a.open_entry(&entry);
    assert_eq!(a.tab_mut().state.url, "http://old/x");
    assert_eq!(a.tab_mut().state.variables[0].name, "keep");
    assert!(a.tab_mut().state.insecure_tls);
}

fn remembering_app(store: &MemoryStore, state: PersistedState) -> ApiTesterApp {
    ApiTesterApp::new(state, Some(History::in_memory()), Box::new(store.clone()))
}

#[test]
fn remembered_secrets_survive_a_restart_but_never_reach_the_state_file() {
    let store = MemoryStore::default();
    let mut first = remembering_app(
        &store,
        PersistedState {
            variables: vec![
                Variable { name: "apiKey".into(), value: "SEKRET-VALUE".into(), secret: true, remember: true },
                Variable { name: "forgetful".into(), value: "GONE-AFTER-RESTART".into(), secret: true, remember: false },
                Variable { name: "host".into(), value: "localhost".into(), secret: false, remember: false },
            ],
            remember_bearer: true,
            ..Default::default()
        },
    );
    first.bearer_token = "BEARER-VALUE".into();
    first.sync_secrets();
    assert!(first.secrets_error.is_none());

    // what eframe would write to disk
    let on_disk = serde_json::to_string(&first.tab_mut().state.redacted()).unwrap();
    for secret in ["SEKRET-VALUE", "BEARER-VALUE", "GONE-AFTER-RESTART"] {
        assert!(!on_disk.contains(secret), "{secret} leaked into the state file: {on_disk}");
    }

    let mut second = remembering_app(&store, serde_json::from_str(&on_disk).unwrap());
    second.restore_secrets();
    assert_eq!(second.bearer_token, "BEARER-VALUE");
    assert_eq!(second.tab_mut().state.variables[0].value, "SEKRET-VALUE");
    assert_eq!(second.tab_mut().state.variables[1].value, "", "an un-remembered secret is blank after a restart");
    assert_eq!(second.tab_mut().state.variables[2].value, "localhost");
}

#[test]
fn forgetting_secrets_empties_the_store_and_unticks_everything() {
    let store = MemoryStore::default();
    let mut a = remembering_app(
        &store,
        PersistedState {
            variables: vec![Variable { name: "tok".into(), value: "v".into(), secret: true, remember: true }],
            remember_bearer: true,
            ..Default::default()
        },
    );
    a.bearer_token = "b".into();
    a.sync_secrets();
    assert_eq!(store.data.borrow().len(), 2);

    a.forget_secrets();
    assert!(store.data.borrow().is_empty());
    assert!(!a.tab_mut().state.remember_bearer && !a.tab_mut().state.variables[0].remember);
    assert!(a.secrets_error.is_none());
}

#[test]
fn an_unreadable_store_shows_an_error_keeps_the_data_and_the_ui_still_draws() {
    let store = MemoryStore::default();
    store.data.borrow_mut().insert("var:apiKey".into(), "precious".into());
    *store.fail_reads.borrow_mut() = true;
    let mut a = remembering_app(
        &store,
        PersistedState {
            variables: vec![Variable { name: "apiKey".into(), value: String::new(), secret: true, remember: true }],
            remember_bearer: true,
            ..Default::default()
        },
    );
    a.restore_secrets();
    assert!(a.secrets_error.as_deref().is_some_and(|e| e.contains("locked")));

    for tab in [RequestTab::Auth, RequestTab::Headers, RequestTab::Variables] {
        a.tab_mut().request_tab = tab;
        draw(&mut a);
    }
    a.sync_secrets(); // an autosave right now must not wipe the stored value
    assert_eq!(store.data.borrow()["var:apiKey"], "precious");
}

fn entry(a: &ApiTesterApp, url: &str) -> HistoryEntry {
    let h = a.history.as_ref().unwrap();
    h.insert(&PersistedState { url: url.into(), ..Default::default() }, Some(200), Some(1)).unwrap();
    h.search_recent("", 1).unwrap().remove(0)
}

#[test]
fn opening_from_the_sidebar_reuses_a_blank_tab_then_adds_tabs_and_never_duplicates() {
    let mut a = app(PersistedState { url: String::new(), ..Default::default() });
    let one = entry(&a, "http://one");
    let two = entry(&a, "http://two");
    a.open_entry(&one);
    assert_eq!(a.tabs.len(), 1, "the blank tab is reused");
    a.open_entry(&two);
    assert_eq!((a.tabs.len(), a.active), (2, 1));
    a.open_entry(&one);
    assert_eq!((a.tabs.len(), a.active), (2, 0), "an already open request is switched to, not reopened");
}

#[test]
fn new_and_closed_tabs_keep_session_settings_and_never_leave_zero_tabs() {
    let mut a = app(PersistedState {
        variables: vec![Variable { name: "host".into(), value: "h".into(), secret: false, remember: false }],
        ..Default::default()
    });
    a.new_tab();
    assert_eq!(a.tabs.len(), 2);
    assert_eq!(a.tab().state.url, "");
    assert_eq!(a.tab().state.variables[0].name, "host");
    a.tab_mut().state.variables[0].value = "changed".into();
    a.activate(0);
    assert_eq!(a.tab().state.variables[0].value, "changed", "variables follow you across tabs");
    a.close_tab(0);
    a.close_tab(0);
    assert_eq!(a.tabs.len(), 1);
    assert!(a.tab().is_pristine());
    draw(&mut a);
}

#[test]
fn naming_a_history_row_saves_it_and_titles_its_tab() {
    let mut a = app(PersistedState { url: String::new(), ..Default::default() });
    let e = entry(&a, "http://h/fixtures/1");
    a.open_entry(&e);
    assert_eq!(a.tab().title(), "1");
    a.start_rename(&e, "history");
    draw(&mut a);
    a.commit_rename(e.id, "  Fixture one ");
    assert_eq!(a.saved_entries.len(), 1);
    assert_eq!(a.tab().title(), "Fixture one");
    assert_eq!(a.tab().saved_id, Some(e.id));
    a.unsave(e.id);
    assert!(a.saved_entries.is_empty());
    assert_eq!(a.tab().title(), "1");
}

#[test]
fn renaming_to_an_existing_saved_name_shows_the_conflict() {
    let mut a = app(PersistedState::default());
    let (first, second) = {
        let h = a.history.as_ref().unwrap();
        (
            h.save_new(&PersistedState { url: "http://a".into(), ..Default::default() }, "First").unwrap(),
            h.save_new(&PersistedState { url: "http://b".into(), ..Default::default() }, "Second").unwrap(),
        )
    };

    a.commit_rename(second, "First");

    assert!(
        a.notice.as_ref().unwrap().0.contains("exact name already exists"),
        "{:?}",
        a.notice
    );
    assert_eq!(
        a.history.as_ref().unwrap().get(second).unwrap().unwrap().name.as_deref(),
        Some("Second")
    );
    assert_eq!(a.history.as_ref().unwrap().get(first).unwrap().unwrap().name.as_deref(), Some("First"));
}

#[test]
fn saving_a_new_tab_creates_a_saved_request_and_saving_again_updates_it() {
    let mut a = app(PersistedState { url: "http://h/users".into(), ..Default::default() });
    a.save_active();
    assert_eq!(a.saved_entries.len(), 1);
    assert!(a.renaming.is_some(), "a new save asks for a name right away");
    let id = a.tab().saved_id.unwrap();
    a.tab_mut().state.url = "http://h/users?page=2".into();
    a.save_active();
    assert_eq!(a.saved_entries.len(), 1);
    assert_eq!(a.saved_entries[0].url, "http://h/users?page=2");
    assert_eq!(a.saved_entries[0].id, id);
    assert!(a.history_entries.is_empty(), "saving is not sending");
}

#[test]
fn imported_requests_open_in_a_tab_and_the_dialogs_draw() {
    let mut a = app(PersistedState { url: "http://busy".into(), ..Default::default() });
    a.open_curl_dialog();
    if let ImportDialog::Curl { text, .. } = &mut a.import {
        *text = format!("curl 'https://h/x?q=1' {}", "-H 'X-Long: aaaaaaaaaaaaaaaaaaaa' ".repeat(80));
    }
    draw(&mut a);
    a.open_parsed(crate::curl_import::parse_curl("curl https://h/x?q=1").unwrap());
    assert_eq!(a.tabs.len(), 2);
    assert_eq!(a.tab().state.params[0].key, "q");
    assert!(matches!(a.import, ImportDialog::Closed));
}

#[test]
fn open_tabs_survive_a_restart_without_their_secrets() {
    let mut a = app(PersistedState { url: "http://one".into(), ..Default::default() });
    a.new_tab();
    a.tab_mut().state.url = "http://two?api_key=SECRET".into();
    a.tab_mut().name = Some("Two".into());
    let open = OpenTabs { tabs: a.tabs.iter().map(Tab::to_saved).collect(), active: a.active };
    let json = serde_json::to_string(&open).unwrap();
    assert!(!json.contains("SECRET"));

    let mut b = app(PersistedState::default());
    b.restore_tabs(serde_json::from_str(&json).unwrap());
    assert_eq!((b.tabs.len(), b.active), (2, 1));
    assert_eq!(b.tab().title(), "Two");
    assert_eq!(b.tabs[0].state.url, "http://one");
}

#[test]
fn requests_an_agent_sends_show_up_in_the_window_on_their_own() {
    let path = std::env::temp_dir().join(format!("plunger-live-{}.sqlite3", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut a = ApiTesterApp::new(PersistedState::default(), Some(History::open_at(&path)), Box::new(MemoryStore::default()));
    assert!(a.history_entries.is_empty());

    // A separate connection, as `plunger mcp` would have.
    let agent = History::open_at(&path);
    agent
        .insert_from(&PersistedState { url: "http://agent/x".into(), ..Default::default() }, Some(200), Some(3), crate::history::Source::Mcp)
        .unwrap();

    a.last_db_poll = Instant::now() - DB_POLL * 2;
    a.poll_database(&egui::Context::default());
    assert_eq!(a.history_entries.len(), 1);
    assert_eq!(a.history_entries[0].source, crate::history::Source::Mcp);
    draw(&mut a); // the agent tag renders
}

#[test]
fn both_themes_draw() {
    let mut a = app(busy_state());
    for choice in [ThemeChoice::Light, ThemeChoice::Dark] {
        let ctx = egui::Context::default();
        theme::apply_theme(&ctx, choice);
        let _ = ctx.run(egui::RawInput::default(), |ctx| a.render_ui(ctx));
    }
}

#[test]
fn the_sidebar_filter_narrows_history_and_saved() {
    let mut a = app(PersistedState::default());
    let h = a.history.as_ref().unwrap();
    for url in ["http://api/users", "http://api/orders"] {
        h.insert(&PersistedState { url: url.into(), ..Default::default() }, Some(200), Some(1)).unwrap();
    }
    h.save_new(&PersistedState { url: "http://api/login".into(), ..Default::default() }, "Sign in").unwrap();
    a.refresh_lists();
    assert_eq!((a.history_entries.len(), a.saved_entries.len()), (2, 1));

    a.sidebar_filter = "orders".into();
    a.refresh_lists();
    assert_eq!((a.history_entries.len(), a.saved_entries.len()), (1, 0));

    a.sidebar_filter = "sign".into();
    a.refresh_lists();
    assert_eq!(a.saved_entries.len(), 1);

    a.sidebar_filter.clear();
    a.refresh_lists();
    assert_eq!(a.history_entries.len(), 2);
}
