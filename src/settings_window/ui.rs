use std::time::{Duration, Instant};

use eframe::egui;
use serde_json::json;

use super::capture::Hint;
use super::{place_on_slot, App, DisplayView, HelperState, RoomView, Target, UiEvent};
use crate::emotes::{catalog, emote_name, DEFAULT_CLICK_EMOTE, DEFAULT_EMOTE_WHEEL, MAX_CUSTOM_EMOTES};
use crate::i18n::{language_name, LanguagePref, Strings, LANG_EN, LANG_ZH_CN, LANGUAGE_PREFS};
use crate::keys::hotkey_label;
use crate::pings::{ALL_PINGS, DEFAULT_CLICK_PING, DEFAULT_WHEEL};
use crate::room::colors::TAG_COLORS;
use crate::room::manager::{Member, Path, Phase};
use crate::room::transport::Link;
use crate::settings::model::{self, EmoteTrigger, Limit, NamedTrigger, Settings, TriggerKey, KEYBOARD_TRIGGERS, LIMITS};

const COLOR_WARN: egui::Color32 = egui::Color32::from_rgb(0xff, 0x8a, 0x65);
const COPIED_FOR: Duration = Duration::from_millis(1500);

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let t = app.t();
            ui.heading(t.page_title);
            ui.add_space(6.0);
            status_card(app, ui, t);
            general(app, ui, t);
            pings_section(app, ui, t);
            wheel_section(app, ui, t);
            emotes_section(app, ui, t);
            room_section(app, ui, t);
            app_section(app, ui, t);
            about_section(app, ui, t);
        });
    });
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(12.0);
    ui.heading(title);
    ui.add_space(2.0);
}

fn row(ui: &mut egui::Ui, title: &str, desc: Option<&str>, add: impl FnOnce(&mut egui::Ui)) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.strong(title);
                if let Some(desc) = desc {
                    ui.weak(desc);
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
        });
    });
}

fn switch_row(app: &mut App, ui: &mut egui::Ui, t: &'static Strings, title: &str, desc: Option<&str>, key: &str, value: bool) {
    let mut checked = value;
    let mut changed = false;
    row(ui, title, desc, |ui| {
        let label = if checked { t.on } else { t.off };
        changed = ui.checkbox(&mut checked, label).changed();
    });
    if changed {
        app.edit(key, json!(checked));
    }
}

fn slider_row(app: &mut App, ui: &mut egui::Ui, title: &str, desc: Option<&str>, key: &str, limit: Limit, value: f64, decimals: usize, suffix: &str) {
    let mut value = value;
    let mut changed = false;
    row(ui, title, desc, |ui| {
        ui.spacing_mut().slider_width = 180.0;
        let slider = egui::Slider::new(&mut value, limit.min..=limit.max).step_by(limit.step).fixed_decimals(decimals).suffix(suffix);
        changed = ui.add(slider).changed();
    });
    if changed {
        app.edit(key, json!(value));
    }
}

fn note(ui: &mut egui::Ui, app: &App) {
    if let Some(text) = &app.note {
        ui.colored_label(COLOR_WARN, text.as_str());
    }
}

fn status_card(app: &mut App, ui: &mut egui::Ui, t: &'static Strings) {
    let enabled = app.status.enabled;
    let helper = app.status.helper;
    let trigger = model::trigger_label(&app.settings.trigger, t.lang);
    let hotkey = hotkey_label(app.settings.toggle_hotkey);
    let (title, hint) = match helper {
        HelperState::Failed => (t.status_helper_stopped, t.status_failed_hint.to_string()),
        HelperState::NoAccess { stale: false } => (t.perm_title, t.perm_steps.to_string()),
        HelperState::NoAccess { stale: true } => (t.perm_stale_title, t.perm_stale_steps.to_string()),
        HelperState::Running => (if enabled { t.status_on } else { t.status_off }, t.status_hint(&trigger, &hotkey, enabled)),
    };
    row(ui, title, Some(hint.as_str()), |ui| match helper {
        HelperState::Failed => {
            if ui.button(t.restart_helper).clicked() {
                let _ = app.events.send(UiEvent::RetryHelper);
            }
        }
        HelperState::NoAccess { stale } => {
            if ui.button(t.perm_open).clicked() {
                let _ = app.events.send(UiEvent::OpenAccessibility);
            }
            if stale && ui.button(t.perm_retry).clicked() {
                let _ = app.events.send(UiEvent::RetryHelper);
            }
        }
        HelperState::Running => {
            let mut on = enabled;
            let label = if on { t.on } else { t.off };
            if ui.checkbox(&mut on, label).changed() {
                let _ = app.events.send(UiEvent::ToggleEnabled(on));
            }
        }
    });
}

fn general(app: &mut App, ui: &mut egui::Ui, t: &'static Strings) {
    let off = !app.status.enabled || app.status.helper == HelperState::Failed;
    section(ui, t.nav_trigger);
    let trigger = app.settings.trigger;
    let mouse = matches!(trigger, TriggerKey::Named(NamedTrigger::Mouse4) | TriggerKey::Named(NamedTrigger::Mouse5));
    let custom = matches!(trigger, TriggerKey::Vk { .. });
    let trigger_desc = if mouse { t.trigger_mouse_desc.to_string() } else { t.trigger_key_desc.to_string() };
    ui.add_enabled_ui(!off, |ui| {
        if capturing(app, Target::Trigger) {
            row(ui, t.trigger_key, Some(trigger_desc.as_str()), |ui| capture_row(app, ui, t));
        } else {
            row(ui, t.trigger_key, Some(trigger_desc.as_str()), |ui| trigger_picker(app, ui, t, false));
        }
        if custom {
            ui.weak(t.custom_key_note);
        }
    });
    note(ui, app);

    let drag = app.settings.drag_threshold_px as f64;
    ui.add_enabled_ui(!off, |ui| {
        slider_row(app, ui, t.drag_distance, Some(t.drag_distance_desc), "dragThresholdPx", LIMITS.drag_threshold_px, drag, 0, " px");
    });

    let trigger_label = model::trigger_label(&app.settings.trigger, t.lang);
    let click_title = t.click_ping(&trigger_label);
    let click_desc = t.click_ping_desc(&trigger_label);
    let click_ping = app.settings.click_ping;
    let game_check = app.settings.game_check;
    ui.add_enabled_ui(!off, |ui| {
        switch_row(app, ui, t, &click_title, Some(click_desc.as_str()), "clickPing", click_ping);
        switch_row(app, ui, t, t.game_check, Some(t.game_check_desc), "gameCheck", game_check);
    });

    section(ui, t.nav_toggle);
    let hotkey = hotkey_label(app.settings.toggle_hotkey);
    if capturing(app, Target::Toggle) {
        row(ui, t.toggle_shortcut, Some(t.toggle_shortcut_desc), |ui| capture_row(app, ui, t));
    } else {
        row(ui, t.toggle_shortcut, Some(t.toggle_shortcut_desc), |ui| {
            if ui.button(&hotkey).clicked() {
                app.start_capture(Target::Toggle);
            }
        });
    }
    let enabled_on_start = app.settings.enabled_on_start;
    switch_row(app, ui, t, t.start_enabled, Some(t.start_enabled_desc), "enabledOnStart", enabled_on_start);
}

fn pings_section(app: &mut App, ui: &mut egui::Ui, t: &'static Strings) {
    section(ui, t.nav_pings);
    let size = app.settings.ping_size_px as f64;
    slider_row(app, ui, t.ping_size, None, "pingSizePx", LIMITS.ping_size_px, size, 0, " px");
    let duration = app.settings.ping_duration_s;
    slider_row(app, ui, t.ping_duration, Some(t.ping_duration_desc), "pingDurationS", LIMITS.ping_duration_s, duration, 1, " s");
    let muted = app.settings.muted;
    let volume = app.settings.volume as f64;
    ui.add_enabled_ui(!muted, |ui| {
        slider_row(app, ui, t.volume, None, "volume", LIMITS.volume, volume, 0, "");
    });
    switch_row(app, ui, t, t.mute, None, "muted", muted);
    let tick = app.settings.tick_sound;
    switch_row(app, ui, t, t.tick_sound, Some(t.tick_sound_desc), "tickSound", tick);
}

fn wheel_section(app: &mut App, ui: &mut egui::Ui, t: &'static Strings) {
    section(ui, t.nav_wheel);
    let trigger = model::trigger_label(&app.settings.trigger, t.lang);
    let pool: Vec<(String, String)> = ALL_PINGS.iter().map(|p| (p.id.to_string(), ping_name(t, p.id))).collect();
    let center_name = name_in(&pool, app.settings.click_ping_id).to_string();
    let wheel = Wheel {
        emote: false,
        title: t.wheel_title,
        desc: t.wheel_desc(&trigger),
        pool_title: t.all_pings,
        pool_desc: t.all_pings_desc,
        ids: app.settings.wheel.iter().map(|id| id.to_string()).collect(),
        center: app.settings.click_ping_id.to_string(),
        key: "wheel",
        center_key: "clickPingId",
        default_ids: DEFAULT_WHEEL.to_vec(),
        default_center: DEFAULT_CLICK_PING,
        center_label: t.wheel_center_label(&trigger, &center_name),
        pool,
    };
    wheel_editor(app, ui, t, &wheel);
}

fn emotes_section(app: &mut App, ui: &mut egui::Ui, t: &'static Strings) {
    section(ui, t.nav_emotes);
    let emote_trigger = app.settings.emote_trigger;
    let key_off = emote_trigger == EmoteTrigger::Off;
    let custom = matches!(emote_trigger, EmoteTrigger::Key(TriggerKey::Vk { .. }));
    let key = match emote_trigger {
        EmoteTrigger::Off => t.emote_key.to_string(),
        EmoteTrigger::Key(key) => model::trigger_label(&key, t.lang),
    };
    let key_desc = if key_off { t.emote_key_off_desc.to_string() } else { t.emote_key_desc.to_string() };
    if capturing(app, Target::Emote) {
        row(ui, t.emote_key, Some(key_desc.as_str()), |ui| capture_row(app, ui, t));
    } else {
        row(ui, t.emote_key, Some(key_desc.as_str()), |ui| trigger_picker(app, ui, t, true));
    }
    if custom {
        ui.weak(t.custom_key_note);
    }
    note(ui, app);

    ui.add_enabled_ui(!key_off, |ui| {
        let click_title = t.emote_click(&key);
        let click_desc = t.emote_click_desc(&key);
        let click = app.settings.emote_click;
        switch_row(app, ui, t, &click_title, Some(click_desc.as_str()), "emoteClick", click);
        let size = app.settings.emote_size_px as f64;
        slider_row(app, ui, t.emote_size, None, "emoteSizePx", LIMITS.emote_size_px, size, 0, " px");
        let sound = app.settings.emote_sound;
        switch_row(app, ui, t, t.emote_sound, Some(t.emote_sound_desc), "emoteSound", sound);
    });

    let pool = emote_pool(&app.settings, t);
    let center_name = name_in(&pool, &app.settings.click_emote_id).to_string();
    let wheel = Wheel {
        emote: true,
        title: t.emote_wheel_title,
        desc: t.emote_wheel_desc(&key),
        pool_title: t.all_emotes,
        pool_desc: t.all_emotes_desc,
        ids: app.settings.emote_wheel.clone(),
        center: app.settings.click_emote_id.clone(),
        key: "emoteWheel",
        center_key: "clickEmoteId",
        default_ids: DEFAULT_EMOTE_WHEEL.to_vec(),
        default_center: DEFAULT_CLICK_EMOTE,
        center_label: t.wheel_center_label(&key, &center_name),
        pool,
    };
    wheel_editor(app, ui, t, &wheel);
    importer(app, ui, t);
}

fn room_section(app: &mut App, ui: &mut egui::Ui, t: &'static Strings) {
    section(ui, t.nav_room);
    let mut commit_name = false;
    row(ui, t.room_name, Some(t.room_name_desc), |ui| {
        let response = ui.add(egui::TextEdit::singleline(&mut app.name_draft).desired_width(180.0));
        if response.changed() {
            app.name_editing = true;
        }
        if response.lost_focus() && app.name_editing {
            commit_name = true;
        }
    });
    if commit_name {
        app.name_editing = false;
        let draft = app.name_draft.clone();
        if draft != app.settings.display_name {
            app.edit("displayName", json!(draft));
        }
    }

    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.strong(t.room_color);
            let selected = app.settings.tag_color;
            for (index, hex) in TAG_COLORS.iter().enumerate() {
                let color_index = index as u32;
                let mut button = egui::Button::new("").fill(hex_color(hex)).min_size(egui::vec2(22.0, 22.0));
                if color_index == selected {
                    button = button.stroke(egui::Stroke::new(2.0, egui::Color32::WHITE));
                }
                if ui.add(button).on_hover_text(t.room_color_label(index + 1)).clicked() {
                    app.edit("tagColor", json!(color_index));
                }
            }
        });
    });

    match app.room.clone() {
        Some(view) if view.state.phase != Phase::Idle => in_room(app, ui, t, &view),
        _ => join_card(app, ui, t),
    }
    if let Some(at) = app.copied_until {
        let now = Instant::now();
        if now < at {
            ui.ctx().request_repaint_after(at - now);
        } else {
            app.copied_until = None;
        }
    }

    let room_muted = app.settings.room_muted;
    switch_row(app, ui, t, t.room_mute, Some(t.room_mute_desc), "roomMuted", room_muted);

    let limit = app.settings.incoming_ping_limit;
    let max = LIMITS.incoming_ping_limit.max;
    let mut limit_value = if limit == 0 { max + 1.0 } else { limit as f64 };
    let mut changed = false;
    ui.add_enabled_ui(!room_muted, |ui| {
        row(ui, t.room_limit, Some(t.room_limit_desc), |ui| {
            ui.spacing_mut().slider_width = 160.0;
            changed = ui.add(egui::Slider::new(&mut limit_value, 1.0..=max + 1.0).step_by(1.0).fixed_decimals(0)).changed();
            ui.weak(t.room_limit_value(limit as usize));
        });
    });
    if changed {
        app.edit("incomingPingLimit", json!(if limit_value > max { 0.0 } else { limit_value }));
    }

    let internet = app.settings.allow_internet;
    switch_row(app, ui, t, t.room_internet, Some(t.room_internet_desc), "allowInternet", internet);
    let rejoin = app.settings.rejoin_room;
    switch_row(app, ui, t, t.room_rejoin, None, "rejoinRoom", rejoin);
    if let Some(view) = app.room.clone() {
        let list = display_list(&view.displays, t);
        row(ui, t.room_displays, Some(t.room_displays_desc), |ui| {
            ui.weak(list);
        });
    }
    ui.weak(format!("{} {}", t.room_privacy, t.room_help));
}

fn join_card(app: &mut App, ui: &mut egui::Ui, t: &'static Strings) {
    let mut join = false;
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.strong(t.room_join_title);
                ui.weak(t.room_join_desc);
                ui.weak(t.room_network_hint);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t.room_create).clicked() {
                    let _ = app.events.send(UiEvent::RoomCreate);
                }
                let ready = !app.join_code.trim().is_empty();
                if ui.add_enabled(ready, egui::Button::new(t.room_join)).clicked() {
                    join = true;
                }
                let field = egui::TextEdit::singleline(&mut app.join_code).hint_text("PING-XXXXX-XXXXX").desired_width(190.0);
                if ui.add(field).changed() {
                    app.join_error = None;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) && ui.memory(|m| m.focused().is_some()) {
                    join = ready;
                }
            });
        });
        if let Some(message) = &app.join_error {
            ui.colored_label(COLOR_WARN, message.as_str());
        }
    });
    if join {
        let _ = app.events.send(UiEvent::RoomJoin(app.join_code.clone()));
    }
}

fn in_room(app: &mut App, ui: &mut egui::Ui, t: &'static Strings, view: &RoomView) {
    let state = &view.state;
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.strong(state.code.as_deref().unwrap_or_default());
                ui.weak(if state.phase == Phase::Joining { t.room_joining } else { t.room_share_desc });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t.room_leave).clicked() {
                    let _ = app.events.send(UiEvent::RoomLeave);
                }
                let copied = app.copied_until.is_some_and(|at| Instant::now() < at);
                if ui.button(if copied { t.room_copied } else { t.room_copy }).clicked() {
                    app.copied_until = Some(Instant::now() + COPIED_FOR);
                    let _ = app.events.send(UiEvent::RoomCopyCode);
                }
            });
        });
    });
    if state.lan == Link::Unavailable {
        ui.group(|ui| {
            ui.colored_label(COLOR_WARN, t.room_lan_unavailable);
            ui.weak(t.room_lan_unavailable_desc);
            if ui.button(t.room_retry).clicked() {
                let _ = app.events.send(UiEvent::RoomRetryLan);
            }
        });
    }
    if state.internet == Link::Unavailable {
        ui.group(|ui| {
            ui.colored_label(COLOR_WARN, t.room_internet_unavailable);
            ui.weak(t.room_internet_unavailable_desc);
            if ui.button(t.room_retry).clicked() {
                let _ = app.events.send(UiEvent::RoomRetryInternet);
            }
        });
    }
    ui.group(|ui| {
        for member in &state.members {
            member_row(app, ui, t, member);
        }
        if state.members.len() <= 1 {
            ui.horizontal_wrapped(|ui| {
                let waiting = if state.lonely { format!("{} {}", t.room_waiting, t.room_lonely) } else { t.room_waiting.to_string() };
                ui.weak(waiting);
                if state.lonely && !app.settings.allow_internet {
                    ui.colored_label(COLOR_WARN, t.room_lonely_internet_off);
                }
            });
        }
        if state.clock_skew {
            ui.colored_label(COLOR_WARN, t.room_clock_skew);
        }
    });
}

fn member_row(app: &mut App, ui: &mut egui::Ui, t: &'static Strings, member: &Member) {
    ui.horizontal(|ui| {
        let (square, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
        let color = TAG_COLORS.get(member.color as usize).copied().unwrap_or(TAG_COLORS[0]);
        ui.painter().rect_filled(square, 2.0, hex_color(color));
        ui.label(&member.name);
        if member.me {
            ui.weak(format!("({})", t.room_you));
        }
        if member.no_emotes {
            ui.weak(t.member_old_no_emotes);
        }
        if !member.me {
            ui.weak(path_label(member.path, t));
        }
        if let Some(badge) = status_badge(&member.status, t) {
            ui.weak(badge);
        }
        if member.needs_update {
            ui.colored_label(COLOR_WARN, t.room_needs_update);
        }
        if member.me {
            return;
        }
        let label = if member.muted { t.room_unmute_member } else { t.room_mute_member };
        let mut button = egui::Button::new(label);
        if member.muted {
            button = button.fill(ui.visuals().selection.bg_fill);
        }
        if ui.add(button).clicked() {
            let _ = app.events.send(UiEvent::RoomMute { peer: member.peer.clone(), on: !member.muted });
        }
    });
}

fn path_label(path: Path, t: &Strings) -> &'static str {
    match path {
        Path::Lan => t.room_path.lan,
        Path::Internet => t.room_path.internet,
        Path::Connecting => t.room_path.connecting,
        Path::Unreachable => t.room_path.unreachable,
    }
}

fn status_badge(status: &str, t: &Strings) -> Option<&'static str> {
    match status {
        "paused" => Some(t.room_paused),
        "muted" => Some(t.room_muted_badge),
        _ => None,
    }
}

fn display_list(displays: &[DisplayView], t: &Strings) -> String {
    displays
        .iter()
        .map(|d| {
            if d.primary {
                format!("#{} {} · {}×{}", d.number, t.room_primary, d.width, d.height)
            } else {
                format!("#{} · {}×{}", d.number, d.width, d.height)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn app_section(app: &mut App, ui: &mut egui::Ui, t: &'static Strings) {
    section(ui, t.nav_app);
    let startup = app.settings.launch_at_startup;
    switch_row(app, ui, t, t.launch_at_startup, Some(t.launch_at_startup_desc), "launchAtStartup", startup);

    let current = app.settings.language;
    let mut chosen = current;
    row(ui, t.language, Some(t.language_desc), |ui| {
        egui::ComboBox::from_id_salt("language").selected_text(lang_label(t, chosen)).width(170.0).show_ui(ui, |ui| {
            for pref in LANGUAGE_PREFS {
                ui.selectable_value(&mut chosen, pref, lang_label(t, pref));
            }
        });
    });
    if chosen != current {
        app.edit("language", json!(lang_value(chosen)));
    }
}

fn about_section(app: &App, ui: &mut egui::Ui, t: &'static Strings) {
    section(ui, t.about);
    ui.weak(t.about_desc(env!("CARGO_PKG_VERSION")));
    if !app.status.problems.is_empty() {
        ui.add_space(6.0);
        ui.strong(t.problems);
        for problem in &app.status.problems {
            ui.colored_label(COLOR_WARN, problem.as_str());
        }
    }
    ui.add_space(6.0);
    ui.strong(t.known_limitations);
    for limitation in t.limitations {
        ui.weak(*limitation);
    }
}

fn importer(app: &mut App, ui: &mut egui::Ui, t: &'static Strings) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            if ui.button(t.add_image).clicked() {
                app.import_file();
            }
            let count = app.settings.custom_emotes.len();
            ui.weak(t.your_images(count, MAX_CUSTOM_EMOTES));
        });
        ui.weak(t.add_image_desc);
        if let Some(message) = app.import.clone() {
            ui.colored_label(COLOR_WARN, message);
        }
        for emote in app.settings.custom_emotes.clone() {
            ui.horizontal(|ui| {
                ui.label(emote.name.as_str());
                if ui.button(t.remove_image(&emote.name)).clicked() {
                    let _ = app.events.send(UiEvent::RemoveEmote(emote.id.clone()));
                }
            });
        }
    });
}

struct Wheel {
    emote: bool,
    title: &'static str,
    desc: String,
    pool_title: &'static str,
    pool_desc: &'static str,
    pool: Vec<(String, String)>,
    ids: Vec<String>,
    center: String,
    key: &'static str,
    center_key: &'static str,
    default_ids: Vec<&'static str>,
    default_center: &'static str,
    center_label: String,
}

fn wheel_editor(app: &mut App, ui: &mut egui::Ui, t: &'static Strings, wheel: &Wheel) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.strong(wheel.title);
                ui.weak(wheel.desc.as_str());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t.wheel_reset).clicked() {
                    let mut patch = super::patch_of(wheel.key, json!(wheel.default_ids));
                    patch.insert(wheel.center_key.to_string(), json!(wheel.default_center));
                    app.send_patch(patch);
                }
            });
        });
        ui.add_space(6.0);
        for (slot, id) in wheel.ids.iter().enumerate() {
            let name = name_in(&wheel.pool, id);
            let label = t.wheel_slot_label(t.slot_names[slot], name);
            if let Some(picked) = combo_row(ui, &label, (wheel.key, slot), &wheel.pool, id) {
                let ids = place_on_slot(&wheel.ids, &picked, slot);
                app.edit(wheel.key, json!(ids));
            }
        }
        if let Some(picked) = combo_row(ui, &wheel.center_label, (wheel.key, "center"), &wheel.pool, &wheel.center) {
            app.edit(wheel.center_key, json!(picked));
        }
        ui.add_space(6.0);
        ui.strong(wheel.pool_title);
        ui.weak(wheel.pool_desc);
        ui.horizontal_wrapped(|ui| {
            for (id, name) in &wheel.pool {
                if ui.button(name.as_str()).clicked() {
                    let event = if wheel.emote {
                        UiEvent::PreviewEmote(id.clone())
                    } else {
                        UiEvent::PreviewPing(id.clone())
                    };
                    let _ = app.events.send(event);
                }
            }
        });
    });
}

fn combo_row(ui: &mut egui::Ui, label: &str, salt: impl std::hash::Hash + std::fmt::Debug, items: &[(String, String)], selected: &str) -> Option<String> {
    let text = name_in(items, selected).to_string();
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(salt).selected_text(text).width(220.0).show_ui(ui, |ui| {
            for (id, name) in items {
                if ui.selectable_label(id == selected, name.as_str()).clicked() && id != selected {
                    picked = Some(id.clone());
                }
            }
        });
    });
    picked
}

fn name_in<'a>(items: &'a [(String, String)], id: &str) -> &'a str {
    items.iter().find(|(item, _)| item == id).map(|(_, name)| name.as_str()).unwrap_or("?")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Off,
    Named(NamedTrigger),
    Custom,
}

fn capturing(app: &App, target: Target) -> bool {
    matches!(app.capture, Some(capture) if capture.target == target)
}

fn capture_row(app: &mut App, ui: &mut egui::Ui, t: &'static Strings) {
    if ui.button(t.press_key).clicked() {
        app.stop_capture();
    }
    if let Some(hint) = app.capture.and_then(|capture| capture.hint) {
        ui.colored_label(COLOR_WARN, hint_text(t, hint));
    }
}

fn hint_text(t: &Strings, hint: Hint) -> &'static str {
    match hint {
        Hint::Unusable => t.hint_unusable,
        Hint::NotTrigger => t.hint_not_trigger,
        Hint::NeedModifier => t.hint_need_modifier,
    }
}

fn trigger_picker(app: &mut App, ui: &mut egui::Ui, t: &'static Strings, emote: bool) {
    let current = if emote { app.settings.emote_trigger } else { EmoteTrigger::Key(app.settings.trigger) };
    let blocked = if emote { EmoteTrigger::Key(app.settings.trigger) } else { app.settings.emote_trigger };
    let start = match current {
        EmoteTrigger::Off => Choice::Off,
        EmoteTrigger::Key(TriggerKey::Named(named)) => Choice::Named(named),
        EmoteTrigger::Key(TriggerKey::Vk { .. }) => Choice::Custom,
    };
    let mut choice = start;
    let text = match current {
        EmoteTrigger::Off => t.emote_key_off.to_string(),
        EmoteTrigger::Key(key) => model::trigger_label(&key, t.lang),
    };
    let id = if emote { "emote-key" } else { "ping-trigger" };
    egui::ComboBox::from_id_salt(id).selected_text(text).width(180.0).show_ui(ui, |ui| {
        if emote {
            ui.selectable_value(&mut choice, Choice::Off, t.emote_key_off);
        }
        ui.label(t.keyboard);
        for named in KEYBOARD_TRIGGERS {
            let taken = model::same_trigger(EmoteTrigger::Key(TriggerKey::Named(named)), blocked);
            let label = model::trigger_label(&TriggerKey::Named(named), t.lang);
            ui.add_enabled_ui(!taken, |ui| {
                ui.selectable_value(&mut choice, Choice::Named(named), label);
            });
        }
        ui.label(t.mouse);
        for (named, label) in [(NamedTrigger::Mouse4, t.mouse4), (NamedTrigger::Mouse5, t.mouse5)] {
            let taken = model::same_trigger(EmoteTrigger::Key(TriggerKey::Named(named)), blocked);
            ui.add_enabled_ui(!taken, |ui| {
                ui.selectable_value(&mut choice, Choice::Named(named), label);
            });
        }
        ui.selectable_value(&mut choice, Choice::Custom, t.custom_key_menu);
    });
    if choice == start {
        return;
    }
    match choice {
        Choice::Off => app.edit("emoteTrigger", json!("off")),
        Choice::Named(named) => app.edit(if emote { "emoteTrigger" } else { "trigger" }, json!(named.as_str())),
        Choice::Custom => app.start_capture(if emote { Target::Emote } else { Target::Trigger }),
    }
}

fn ping_name(t: &Strings, id: &str) -> String {
    let names = &t.ping_names;
    match id {
        "danger" => names.danger,
        "push" => names.push,
        "omw" => names.omw,
        "allin" => names.allin,
        "assist" => names.assist,
        "needvision" => names.needvision,
        "missing" => names.missing,
        "enemyvision" => names.enemyvision,
        "generic" => names.generic,
        "bait" => names.bait,
        "visioncleared" => names.visioncleared,
        _ => id,
    }
    .to_string()
}

fn emote_pool(settings: &Settings, t: &Strings) -> Vec<(String, String)> {
    let mut pool: Vec<(String, String)> =
        catalog::EMOTES.iter().map(|emote| (emote.slug.to_string(), emote_name(emote.slug, t.lang, &[]))).collect();
    pool.extend(settings.custom_emotes.iter().map(|custom| (custom.id.clone(), custom.name.clone())));
    pool
}

fn hex_color(hex: &str) -> egui::Color32 {
    let value = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    egui::Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

fn lang_label(t: &Strings, pref: LanguagePref) -> &'static str {
    match pref {
        LanguagePref::Auto => t.language_auto,
        LanguagePref::En => language_name(LANG_EN),
        LanguagePref::ZhCn => language_name(LANG_ZH_CN),
    }
}

fn lang_value(pref: LanguagePref) -> &'static str {
    match pref {
        LanguagePref::Auto => "auto",
        LanguagePref::En => "en",
        LanguagePref::ZhCn => "zh-CN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::strings;

    fn en() -> &'static Strings {
        strings(LANG_EN)
    }

    #[test]
    fn paths_map_to_their_label() {
        let t = en();
        assert_eq!(path_label(Path::Lan, t), "LAN");
        assert_eq!(path_label(Path::Internet, t), "Internet");
        assert_eq!(path_label(Path::Connecting, t), t.room_path.connecting);
        assert_eq!(path_label(Path::Unreachable, t), t.room_path.unreachable);
    }

    #[test]
    fn statuses_map_to_their_badge() {
        let t = en();
        assert_eq!(status_badge("paused", t), Some(t.room_paused));
        assert_eq!(status_badge("muted", t), Some(t.room_muted_badge));
        assert_eq!(status_badge("on", t), None);
    }

    #[test]
    fn displays_list_numbers_sizes_and_marks_the_primary() {
        let t = en();
        let list = [
            DisplayView { number: 1, width: 1920, height: 1080, primary: true },
            DisplayView { number: 2, width: 2560, height: 1440, primary: false },
        ];
        assert_eq!(display_list(&list, t), format!("#1 {} · 1920×1080 #2 · 2560×1440", t.room_primary));
        assert_eq!(display_list(&[], t), "");
    }
}
