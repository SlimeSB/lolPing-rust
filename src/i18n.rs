pub type Lang = &'static str;

pub const LANG_EN: Lang = "en";
pub const LANG_ZH_CN: Lang = "zh-CN";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanguagePref {
    Auto,
    En,
    ZhCn,
}

pub const LANGUAGE_PREFS: [LanguagePref; 3] = [LanguagePref::Auto, LanguagePref::En, LanguagePref::ZhCn];

pub fn language_name(lang: Lang) -> &'static str {
    if lang == LANG_ZH_CN {
        "简体中文"
    } else {
        "English"
    }
}

pub fn resolve_lang(pref: LanguagePref, locale: &str) -> Lang {
    match pref {
        LanguagePref::En => LANG_EN,
        LanguagePref::ZhCn => LANG_ZH_CN,
        LanguagePref::Auto => {
            if locale.len() >= 2 && locale[..2].eq_ignore_ascii_case("zh") {
                LANG_ZH_CN
            } else {
                LANG_EN
            }
        }
    }
}

pub struct RoomPathStrings {
    pub lan: &'static str,
    pub internet: &'static str,
    pub connecting: &'static str,
    pub unreachable: &'static str,
}

pub struct PingNameStrings {
    pub danger: &'static str,
    pub push: &'static str,
    pub omw: &'static str,
    pub allin: &'static str,
    pub assist: &'static str,
    pub needvision: &'static str,
    pub missing: &'static str,
    pub enemyvision: &'static str,
    pub generic: &'static str,
    pub bait: &'static str,
    pub visioncleared: &'static str,
}

macro_rules! strings_table {
    ($($name:ident: $ty:ty),* $(,)?) => {
        pub struct Strings {
            $(pub $name: $ty),*
        }

        impl Strings {
            // The reference Strings type has 215 keys; 37 of them are functions, which are methods here.
            pub const FIELD_COUNT: usize = [$(stringify!($name)),*].len();
        }
    };
}

strings_table! {
    lang: &'static str,
    on: &'static str,
    off: &'static str,
    nav_trigger: &'static str,
    nav_toggle: &'static str,
    nav_pings: &'static str,
    nav_wheel: &'static str,
    nav_emotes: &'static str,
    nav_room: &'static str,
    nav_app: &'static str,
    tagline: &'static str,
    page_title: &'static str,
    status_helper_stopped: &'static str,
    status_on: &'static str,
    status_off: &'static str,
    status_failed_hint: &'static str,
    restart_helper: &'static str,
    trigger_key: &'static str,
    trigger_key_desc: &'static str,
    trigger_mouse_desc: &'static str,
    custom_key_note: &'static str,
    drag_distance: &'static str,
    drag_distance_desc: &'static str,
    toggle_shortcut: &'static str,
    toggle_shortcut_desc: &'static str,
    start_enabled: &'static str,
    start_enabled_desc: &'static str,
    ping_size: &'static str,
    ping_duration: &'static str,
    ping_duration_desc: &'static str,
    volume: &'static str,
    mute: &'static str,
    tick_sound: &'static str,
    tick_sound_desc: &'static str,
    wheel_title: &'static str,
    wheel_reset: &'static str,
    all_pings: &'static str,
    all_pings_desc: &'static str,
    wheel_center: &'static str,
    wheel_new: &'static str,
    slot_names: &'static [&'static str],
    emote_key: &'static str,
    emote_key_desc: &'static str,
    emote_key_off: &'static str,
    emote_key_off_desc: &'static str,
    emote_size: &'static str,
    emote_sound: &'static str,
    emote_sound_desc: &'static str,
    emote_wheel_title: &'static str,
    all_emotes: &'static str,
    all_emotes_desc: &'static str,
    add_image: &'static str,
    add_image_desc: &'static str,
    import_err_full: &'static str,
    launch_at_startup: &'static str,
    launch_at_startup_desc: &'static str,
    language: &'static str,
    language_desc: &'static str,
    language_auto: &'static str,
    about: &'static str,
    github_page: &'static str,
    open_settings_folder: &'static str,
    problems: &'static str,
    known_limitations: &'static str,
    press_key: &'static str,
    hint_unusable: &'static str,
    hint_not_trigger: &'static str,
    hint_need_modifier: &'static str,
    keyboard: &'static str,
    mouse: &'static str,
    mouse_back: &'static str,
    mouse_forward: &'static str,
    custom_key: &'static str,
    custom_key_menu: &'static str,
    choose_key: &'static str,
    mouse4: &'static str,
    mouse5: &'static str,
    room_name: &'static str,
    room_name_desc: &'static str,
    room_color: &'static str,
    room_join_title: &'static str,
    room_join_desc: &'static str,
    room_network_hint: &'static str,
    room_join: &'static str,
    room_create: &'static str,
    room_invalid: &'static str,
    room_mistyped: &'static str,
    room_join_failed: &'static str,
    room_share_desc: &'static str,
    room_joining: &'static str,
    room_copy: &'static str,
    room_copied: &'static str,
    room_leave: &'static str,
    room_you: &'static str,
    room_path: RoomPathStrings,
    room_paused: &'static str,
    room_muted_badge: &'static str,
    room_needs_update: &'static str,
    member_old_no_emotes: &'static str,
    room_clock_skew: &'static str,
    room_mute_member: &'static str,
    room_unmute_member: &'static str,
    room_waiting: &'static str,
    room_lonely: &'static str,
    room_lan_unavailable: &'static str,
    room_lan_unavailable_desc: &'static str,
    room_mute: &'static str,
    room_mute_desc: &'static str,
    room_limit: &'static str,
    room_limit_desc: &'static str,
    room_rejoin: &'static str,
    room_internet: &'static str,
    room_internet_desc: &'static str,
    room_internet_unavailable: &'static str,
    room_internet_unavailable_desc: &'static str,
    room_retry: &'static str,
    room_lonely_internet_off: &'static str,
    room_displays: &'static str,
    room_displays_desc: &'static str,
    room_primary: &'static str,
    room_privacy: &'static str,
    room_help: &'static str,
    ping_names: PingNameStrings,
    limitations: &'static [&'static str],
    emote_key_clash: &'static str,
    problem_emote_key_clash: &'static str,
    toast_on: &'static str,
    toast_off: &'static str,
    toast_helper_stopped: &'static str,
    toast_helper_stopped_body: &'static str,
    tray_tip_failed: &'static str,
    tray_enabled: &'static str,
    tray_restart: &'static str,
    tray_settings: &'static str,
    tray_quit: &'static str,
    tray_room: &'static str,
    tray_room_join_clipboard: &'static str,
    tray_room_create: &'static str,
    tray_room_settings: &'static str,
    tray_room_copy: &'static str,
    tray_room_mute: &'static str,
    tray_room_leave: &'static str,
    toast_room_joined_body: &'static str,
    toast_room_full: &'static str,
    toast_room_full_body: &'static str,
    toast_no_code: &'static str,
    toast_no_code_body: &'static str,
    update_now: &'static str,
    whats_new: &'static str,
    check_for_updates: &'static str,
    checking: &'static str,
    up_to_date: &'static str,
    installing_update: &'static str,
    update_toast_title: &'static str,
    update_error_toast_title: &'static str,
    restarting_to_update: &'static str,
    update_mac_steps: &'static str,
    update_mac_dialog: &'static str,
    update_err_download: &'static str,
    update_err_damaged: &'static str,
    update_err_install: &'static str,
    update_reason_unknown: &'static str,
    retry: &'static str,
    auto_update_check: &'static str,
    auto_update_check_desc: &'static str,
    tray_check_for_updates: &'static str,
    tray_installing: &'static str,
}

impl Strings {
    pub fn status_hint(&self, trigger: &str, hotkey: &str, enabled: bool) -> String {
        if self.lang == LANG_ZH_CN {
            format!("按住 {trigger} 并拖动打开信号轮盘 · {hotkey} {}", if enabled { "关闭" } else { "开启" })
        } else {
            format!("Hold {trigger} and drag to open the ping wheel · {hotkey} to turn {}", if enabled { "off" } else { "on" })
        }
    }

    pub fn click_ping(&self, trigger: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("{trigger} + 单击发送信号")
        } else {
            format!("{trigger} + click places a ping")
        }
    }

    pub fn click_ping_desc(&self, trigger: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("发送轮盘中心的信号。关闭时，{trigger} + 单击会直接传给下方的程序")
        } else {
            format!("Places the ping in the centre of the wheel. When off, {trigger} + click passes straight through to the app underneath")
        }
    }

    pub fn wheel_desc(&self, trigger: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("把信号拖到扇区上即可放置。拖到中心则设为 {trigger} + 单击发送的信号。")
        } else {
            format!("Drag a ping onto a slice to put it there. Drop it on the centre to make it the {trigger} + click ping.")
        }
    }

    pub fn wheel_pick_hint(&self, name: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("请单击一个扇区或中心来放置“{name}”。按 Esc 取消。")
        } else {
            format!("Now click a slice or the centre to place {name}. Esc cancels.")
        }
    }

    pub fn wheel_slot_label(&self, dir: &str, name: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("{dir}：{name}")
        } else {
            format!("{dir}: {name}")
        }
    }

    pub fn wheel_center_label(&self, trigger: &str, name: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("{trigger} + 单击：{name}")
        } else {
            format!("{trigger} + click: {name}")
        }
    }

    pub fn emote_click(&self, key: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("{key} + 单击发送表情")
        } else {
            format!("{key} + click places an emote")
        }
    }

    pub fn emote_click_desc(&self, key: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("发送表情轮盘中心的表情。关闭时，{key} + 单击会直接传给下方的程序")
        } else {
            format!("Places the emote in the centre of the emote wheel. When off, {key} + click passes straight through to the app underneath")
        }
    }

    pub fn emote_wheel_desc(&self, key: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("把表情拖到扇区上即可放置。拖到中心则设为 {key} + 单击发送的表情。")
        } else {
            format!("Drag an emote onto a slice to put it there. Drop it on the centre to make it the {key} + click emote.")
        }
    }

    pub fn remove_image(&self, name: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("移除“{name}”")
        } else {
            format!("Remove {name}")
        }
    }

    pub fn your_images(&self, n: usize, max: usize) -> String {
        if self.lang == LANG_ZH_CN {
            format!("我的图片：{n}/{max}")
        } else {
            format!("Your images: {n} of {max}")
        }
    }

    pub fn import_err_type(&self, file: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("“{file}”不是 PNG、JPG、GIF 或 WebP 图片。")
        } else {
            format!("{file} isn’t a PNG, JPG, GIF or WebP image.")
        }
    }

    pub fn import_err_size(&self, file: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("“{file}”太大：图片最大 8 MB，最长边不超过 8192 像素。")
        } else {
            format!("{file} is too big: an image can be up to 8 MB and 8192 pixels on its longest side.")
        }
    }

    pub fn import_err_anim_size(&self, file: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("“{file}”太大，无法作为动图：动图最大 5 MB，最长边不超过 1024 像素。")
        } else {
            format!("{file} is too big to animate: an animation can be up to 5 MB and 1024 pixels on its longest side.")
        }
    }

    pub fn import_err_detail(&self, file: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("“{file}”细节太多，无法做成足够小的表情。请换一张简单些的图片。")
        } else {
            format!("{file} is too detailed to make a small emote from. Try a simpler image.")
        }
    }

    pub fn import_err_duplicate(&self, file: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("“{file}”已在我的图片中。")
        } else {
            format!("{file} is already one of your images.")
        }
    }

    pub fn import_err_disk(&self, file: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("无法保存“{file}”。请检查设置文件夹是否可以写入。")
        } else {
            format!("{file} couldn’t be saved. Check that the settings folder can be written to.")
        }
    }

    pub fn about_desc(&self, version: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("lolPing {version} · 信号图标和音效 © Riot Games")
        } else {
            format!("lolPing {version} · Ping icons and sounds © Riot Games")
        }
    }

    pub fn room_color_label(&self, n: usize) -> String {
        if self.lang == LANG_ZH_CN {
            format!("颜色 {n}")
        } else {
            format!("Colour {n}")
        }
    }

    pub fn room_limit_value(&self, n: usize) -> String {
        if self.lang == LANG_ZH_CN {
            if n == 0 {
            "不限".to_string()
        } else {
            format!("{n} 次/秒")
        }
        } else {
            if n == 0 {
            "Unlimited".to_string()
        } else {
            format!("{n} / s")
        }
        }
    }

    pub fn missing_asset(&self, file: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("缺少素材：{file}")
        } else {
            format!("Missing asset: {file}")
        }
    }

    pub fn hotkey_taken(&self, hotkey: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("{hotkey} 已被其他程序占用。")
        } else {
            format!("{hotkey} is already used by another app.")
        }
    }

    pub fn toast_toggle_hint(&self, hotkey: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("{hotkey} 切换")
        } else {
            format!("{hotkey} to toggle")
        }
    }

    pub fn tray_tip(&self, on: bool) -> String {
        if self.lang == LANG_ZH_CN {
            format!("lolPing：信号{}", if on { "已开启" } else { "已关闭" })
        } else {
            format!("lolPing: pings {}", if on { "on" } else { "off" })
        }
    }

    pub fn tray_room_label(&self, code: &str, n: usize) -> String {
        if self.lang == LANG_ZH_CN {
            format!("{code} · {n} 人")
        } else {
            format!("{code} · {n} {}", if n == 1 { "person" } else { "people" })
        }
    }

    pub fn toast_room_joined(&self, code: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("已加入房间 {code}")
        } else {
            format!("Joined room {code}")
        }
    }

    pub fn toast_member_joined(&self, name: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("{name} 加入了房间")
        } else {
            format!("{name} joined")
        }
    }

    pub fn toast_member_left(&self, name: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("{name} 离开了房间")
        } else {
            format!("{name} left")
        }
    }

    pub fn toast_member_lost(&self, name: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("{name} 已断开连接")
        } else {
            format!("{name} disconnected")
        }
    }

    pub fn update_available(&self, v: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("有新版本 lolPing {v}")
        } else {
            format!("lolPing {v} is available")
        }
    }

    pub fn checked_at(&self, time: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("检查于 {time}")
        } else {
            format!("Checked {time}")
        }
    }

    pub fn downloading_update(&self, n: usize) -> String {
        if self.lang == LANG_ZH_CN {
            format!("正在下载更新… {n}%")
        } else {
            format!("Downloading update… {n}%")
        }
    }

    pub fn update_toast_body(&self, v: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("lolPing {v} 可以下载了。请从托盘菜单或设置中更新。")
        } else {
            format!("lolPing {v} is ready to download. Update from the tray menu or Settings.")
        }
    }

    pub fn update_err_check(&self, reason: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("无法检查更新：{reason}")
        } else {
            format!("Couldn’t check for updates: {reason}")
        }
    }

    pub fn tray_update_to(&self, v: &str) -> String {
        if self.lang == LANG_ZH_CN {
            format!("更新到 {v}…")
        } else {
            format!("Update to {v}…")
        }
    }

    pub fn tray_downloading(&self, n: usize) -> String {
        if self.lang == LANG_ZH_CN {
            format!("正在下载更新… {n}%")
        } else {
            format!("Downloading update… {n}%")
        }
    }
}

static EN: Strings = Strings {
    lang: LANG_EN,
    on: "On",
    off: "Off",
    nav_trigger: "Trigger",
    nav_toggle: "Toggle",
    nav_pings: "Pings & sound",
    nav_wheel: "Wheel",
    nav_emotes: "Emotes",
    nav_room: "Room",
    nav_app: "App",
    tagline: "Ping anywhere",
    page_title: "Settings",
    status_helper_stopped: "Input helper stopped",
    status_on: "Pinging is on",
    status_off: "Pinging is off",
    status_failed_hint: "Mouse and keyboard input can’t be read right now.",
    restart_helper: "Restart helper",
    trigger_key: "Trigger key",
    trigger_key_desc: "Hold this, then drag with the left mouse button to open the wheel",
    trigger_mouse_desc: "Press and drag with this mouse button to open the wheel",
    custom_key_note: "This key won’t type in other apps while lolPing is on",
    drag_distance: "Drag distance",
    drag_distance_desc: "How far the mouse must move before the wheel opens",
    toggle_shortcut: "Enable / disable shortcut",
    toggle_shortcut_desc: "Works anywhere, even while pinging is off",
    start_enabled: "Start enabled",
    start_enabled_desc: "Pinging is on when lolPing launches",
    ping_size: "Ping size",
    ping_duration: "Ping duration",
    ping_duration_desc: "How long a ping stays on screen",
    volume: "Volume",
    mute: "Mute ping sounds",
    tick_sound: "Wheel tick sound",
    tick_sound_desc: "Plays a quiet click when hovering a new slice",
    wheel_title: "Customize the wheel",
    wheel_reset: "Reset to default",
    all_pings: "All pings",
    all_pings_desc: "Click to preview. Drag onto the wheel to assign, or click a ping and then a slot.",
    wheel_center: "CLICK",
    wheel_new: "NEW",
    slot_names: &["N", "NE", "E", "SE", "S", "SW", "W", "NW"],
    emote_key: "Emote key",
    emote_key_desc: "Hold this and drag to open the emote wheel. It can’t be the same as the trigger key.",
    emote_key_off: "Off",
    emote_key_off_desc: "The emote wheel is off. Pick a key to turn it on.",
    emote_size: "Emote size",
    emote_sound: "Emote sounds",
    emote_sound_desc: "Plays an emote’s own sound at the volume set under Pings & sound",
    emote_wheel_title: "Customize the emote wheel",
    all_emotes: "All emotes",
    all_emotes_desc: "Click to preview. Drag onto the wheel to assign, or click an emote and then a slot.",
    add_image: "Add image…",
    add_image_desc: "PNG, JPG, GIF or WebP, up to 8 MB and 8192 pixels on the longest side. You can also drop images on the list.",
    import_err_full: "You can add up to 24 images. Remove one to add another.",
    launch_at_startup: "Launch at Windows startup",
    launch_at_startup_desc: "Starts hidden in the tray",
    language: "Language",
    language_desc: "Follow Windows, or pick one",
    language_auto: "Follow Windows",
    about: "About",
    github_page: "GitHub page",
    open_settings_folder: "Open settings folder",
    problems: "Problems",
    known_limitations: "Known limitations",
    press_key: "Press a key…",
    hint_unusable: "That key can’t be used",
    hint_not_trigger: "That key can’t be used as a trigger",
    hint_need_modifier: "Add Ctrl, Alt or Win",
    keyboard: "Keyboard",
    mouse: "Mouse",
    mouse_back: "back",
    mouse_forward: "forward",
    custom_key: "Custom key",
    custom_key_menu: "Custom key…",
    choose_key: "Choose a key",
    mouse4: "Mouse 4",
    mouse5: "Mouse 5",
    room_name: "Your name",
    room_name_desc: "Shown under your pings on other people’s screens",
    room_color: "Tag colour",
    room_join_title: "Join a room",
    room_join_desc: "Paste the code a friend sent you, or create a room and share yours",
    room_network_hint: "The first time, Windows asks whether lolPing may use your network: allow it, and make sure your Wi‑Fi is set to Private.",
    room_join: "Join",
    room_create: "Create room",
    room_invalid: "That isn’t a room code.",
    room_mistyped: "This code looks mistyped.",
    room_join_failed: "Couldn’t join the room. Try again.",
    room_share_desc: "Share this code with friends",
    room_joining: "Joining…",
    room_copy: "Copy code",
    room_copied: "Copied",
    room_leave: "Leave room",
    room_you: "you",
    room_path: RoomPathStrings { lan: "LAN", internet: "Internet", connecting: "Connecting…", unreachable: "Can’t reach" },
    room_paused: "paused",
    room_muted_badge: "muted",
    room_needs_update: "needs update",
    member_old_no_emotes: "Can’t see emotes, needs an update",
    room_clock_skew: "A device trying to join has a clock more than 10 minutes off. Check the date and time settings on every device.",
    room_mute_member: "Mute",
    room_unmute_member: "Unmute",
    room_waiting: "Waiting for others…",
    room_lonely: "Nobody here yet. Check that your friends typed the same code.",
    room_lan_unavailable: "LAN unavailable",
    room_lan_unavailable_desc: "lolPing couldn’t open UDP port 47474. Another app may be using it; free the port and click Retry.",
    room_mute: "Mute room",
    room_mute_desc: "Keep pinging, but hide everyone else’s pings and emotes",
    room_limit: "Incoming ping and emote limit",
    room_limit_desc: "Pings and emotes together, per person. Unlimited lets friends spam you",
    room_rejoin: "Rejoin last room on launch",
    room_internet: "Allow internet connections",
    room_internet_desc: "Meet friends on other networks through public relays. Off: same network only.",
    room_internet_unavailable: "Internet unavailable",
    room_internet_unavailable_desc: "Can’t reach any relay. Retrying in the background.",
    room_retry: "Retry",
    room_lonely_internet_off: "Friends on another network? Turn on Allow internet connections.",
    room_displays: "Displays",
    room_displays_desc: "A ping lands on the display with the same number, or on #1",
    room_primary: "Primary",
    room_privacy: "Pings and names are end-to-end encrypted. People on your network can see your local IP address. With internet connections on, public relays see your IP address, an anonymous room ID and when you send, but never your pings or your name.",
    room_help: "Can’t see a friend? Check that you both typed the same code. On the same network, lolPing must be allowed through the firewall; on different networks, you both need Allow internet connections on.",
    ping_names: PingNameStrings { danger: "Danger", push: "Push", omw: "On My Way", allin: "All In", assist: "Assist Me", needvision: "Need Vision", missing: "Enemy Missing", enemyvision: "Enemy Vision", generic: "Generic", bait: "Bait", visioncleared: "Vision Cleared" },
    limitations: &["The wheel can’t be triggered over elevated (administrator) windows such as Task Manager.", "Exclusive-fullscreen games draw above the overlay.", "Single-window screen capture doesn’t include the overlay; use whole-screen capture.", "While lolPing is on, Ctrl + drag (the default emote key) opens the emote wheel instead of copying files or text. Change the emote key or turn it off in Settings → Emotes."],
    emote_key_clash: "The ping trigger and the emote key can’t be the same key, and the emote key can’t be the pause shortcut’s key.",
    problem_emote_key_clash: "The emote key was the same as another lolPing key, so it was turned off.",
    toast_on: "Pings on",
    toast_off: "Pings off",
    toast_helper_stopped: "Ping helper stopped",
    toast_helper_stopped_body: "Right-click the tray icon to restart it",
    tray_tip_failed: "lolPing: input helper stopped",
    tray_enabled: "Enabled",
    tray_restart: "Restart input helper",
    tray_settings: "Settings…",
    tray_quit: "Quit lolPing",
    tray_room: "Room",
    tray_room_join_clipboard: "Join from clipboard",
    tray_room_create: "Create room",
    tray_room_settings: "Room settings…",
    tray_room_copy: "Copy code",
    tray_room_mute: "Mute room",
    tray_room_leave: "Leave room",
    toast_room_joined_body: "Share the code with friends",
    toast_room_full: "Room is full",
    toast_room_full_body: "A room holds up to 8 people",
    toast_no_code: "No room code on the clipboard",
    toast_no_code_body: "Copy a code like PING-XXXXX-XXXXX first",
    update_now: "Update",
    whats_new: "What’s new",
    check_for_updates: "Check for updates",
    checking: "Checking…",
    up_to_date: "lolPing is up to date",
    installing_update: "Installing update…",
    update_toast_title: "Update available",
    update_error_toast_title: "Update error",
    restarting_to_update: "Restarting to update…",
    update_mac_steps: "lolPing downloads the update and opens it. Drag lolPing into Applications, choose Replace, then open it and allow Accessibility again.",
    update_mac_dialog: "Drag lolPing into Applications and choose Replace. Then open it and allow Accessibility again. lolPing will quit now.",
    update_err_download: "The download failed. Try again.",
    update_err_damaged: "The download was damaged. Try again.",
    update_err_install: "The update couldn’t be installed. Try again.",
    update_reason_unknown: "no details",
    retry: "Retry",
    auto_update_check: "Check for updates automatically",
    auto_update_check_desc: "Checks GitHub for a new version every few hours. Nothing downloads until you click Update.",
    tray_check_for_updates: "Check for updates",
    tray_installing: "Installing update…",
};

static ZH: Strings = Strings {
    lang: LANG_ZH_CN,
    on: "开",
    off: "关",
    nav_trigger: "触发",
    nav_toggle: "开关",
    nav_pings: "信号与声音",
    nav_wheel: "轮盘",
    nav_emotes: "表情",
    nav_room: "房间",
    nav_app: "应用",
    tagline: "随处发信号",
    page_title: "设置",
    status_helper_stopped: "输入助手已停止",
    status_on: "信号已开启",
    status_off: "信号已关闭",
    status_failed_hint: "目前无法读取鼠标和键盘输入。",
    restart_helper: "重启助手",
    trigger_key: "触发键",
    trigger_key_desc: "按住此键，再用鼠标左键拖动以打开轮盘",
    trigger_mouse_desc: "按住此鼠标键并拖动以打开轮盘",
    custom_key_note: "lolPing 开启时，此键在其他程序中不会输入字符",
    drag_distance: "拖动距离",
    drag_distance_desc: "鼠标移动多远后才打开轮盘",
    toggle_shortcut: "开关快捷键",
    toggle_shortcut_desc: "在任何地方都可用，即使信号已关闭",
    start_enabled: "启动时开启",
    start_enabled_desc: "启动 lolPing 时信号处于开启状态",
    ping_size: "信号大小",
    ping_duration: "信号持续时间",
    ping_duration_desc: "信号在屏幕上停留多久",
    volume: "音量",
    mute: "静音信号声音",
    tick_sound: "轮盘提示音",
    tick_sound_desc: "悬停到新的扇区时播放轻微的点击声",
    wheel_title: "自定义轮盘",
    wheel_reset: "恢复默认",
    all_pings: "全部信号",
    all_pings_desc: "单击可预览。拖到轮盘上即可分配，也可以先单击信号，再单击位置。",
    wheel_center: "单击",
    wheel_new: "新",
    slot_names: &["上", "右上", "右", "右下", "下", "左下", "左", "左上"],
    emote_key: "表情键",
    emote_key_desc: "按住此键并拖动以打开表情轮盘，不能与触发键相同。",
    emote_key_off: "关闭",
    emote_key_off_desc: "表情轮盘已关闭。选择一个键即可开启。",
    emote_size: "表情大小",
    emote_sound: "表情音效",
    emote_sound_desc: "按“信号与声音”中设置的音量播放表情自带的音效",
    emote_wheel_title: "自定义表情轮盘",
    all_emotes: "全部表情",
    all_emotes_desc: "单击可预览。拖到轮盘上即可分配，也可以先单击表情，再单击位置。",
    add_image: "添加图片…",
    add_image_desc: "支持 PNG、JPG、GIF 和 WebP，最大 8 MB，最长边不超过 8192 像素。也可以把图片拖到列表上。",
    import_err_full: "最多可添加 24 张图片。请先移除一张再添加。",
    launch_at_startup: "开机启动",
    launch_at_startup_desc: "启动后隐藏在托盘中",
    language: "语言",
    language_desc: "跟随 Windows，或手动选择",
    language_auto: "跟随 Windows",
    about: "关于",
    github_page: "GitHub 主页",
    open_settings_folder: "打开设置文件夹",
    problems: "问题",
    known_limitations: "已知限制",
    press_key: "请按一个键…",
    hint_unusable: "这个键无法使用",
    hint_not_trigger: "这个键不能用作触发键",
    hint_need_modifier: "请加上 Ctrl、Alt 或 Win",
    keyboard: "键盘",
    mouse: "鼠标",
    mouse_back: "后退",
    mouse_forward: "前进",
    custom_key: "自定义按键",
    custom_key_menu: "自定义按键…",
    choose_key: "选择一个键",
    mouse4: "鼠标侧键 4",
    mouse5: "鼠标侧键 5",
    room_name: "你的名字",
    room_name_desc: "显示在你的信号下方，其他人可以看到",
    room_color: "名字颜色",
    room_join_title: "加入房间",
    room_join_desc: "粘贴朋友发来的房间码，或创建房间并分享你的房间码",
    room_network_hint: "第一次使用时，Windows 会询问是否允许 lolPing 使用网络：请允许，并确认你的 Wi‑Fi 设置为“专用网络”。",
    room_join: "加入",
    room_create: "创建房间",
    room_invalid: "这不是房间码。",
    room_mistyped: "房间码好像输错了。",
    room_join_failed: "无法加入房间，请重试。",
    room_share_desc: "把这个房间码分享给朋友",
    room_joining: "正在加入…",
    room_copy: "复制房间码",
    room_copied: "已复制",
    room_leave: "离开房间",
    room_you: "你",
    room_path: RoomPathStrings { lan: "局域网", internet: "互联网", connecting: "正在连接…", unreachable: "无法连接" },
    room_paused: "已暂停",
    room_muted_badge: "已静音",
    room_needs_update: "需要更新",
    member_old_no_emotes: "看不到表情，需要更新",
    room_clock_skew: "有设备的时钟偏差超过 10 分钟，无法加入。请检查每台设备的日期和时间设置。",
    room_mute_member: "静音",
    room_unmute_member: "取消静音",
    room_waiting: "正在等待其他人…",
    room_lonely: "还没有人加入。请确认朋友输入的是同一个房间码。",
    room_lan_unavailable: "局域网不可用",
    room_lan_unavailable_desc: "lolPing 无法打开 UDP 端口 47474，可能被其他程序占用了。释放该端口后点按“重试”。",
    room_mute: "房间静音",
    room_mute_desc: "你仍可发信号和表情，但不显示其他人的信号和表情",
    room_limit: "接收信号和表情上限",
    room_limit_desc: "信号和表情合计，按每人计算。选择“不限”时朋友可以随意刷屏",
    room_rejoin: "启动时重新加入上次的房间",
    room_internet: "允许互联网连接",
    room_internet_desc: "通过公共中继与其他网络上的朋友连接。关闭后仅限同一网络。",
    room_internet_unavailable: "互联网不可用",
    room_internet_unavailable_desc: "无法连接任何中继，正在后台重试。",
    room_retry: "重试",
    room_lonely_internet_off: "朋友在其他网络？请打开“允许互联网连接”。",
    room_displays: "显示器",
    room_displays_desc: "信号显示在编号相同的显示器上，没有则显示在 1 号上",
    room_primary: "主显示器",
    room_privacy: "信号和名字均为端到端加密。同一网络中的人可以看到你的本地 IP 地址。开启互联网连接后，公共中继可以看到你的 IP 地址、匿名房间 ID 和发送时间，但永远看不到你的信号和名字。",
    room_help: "看不到朋友？请确认你们输入的是同一个房间码。在同一网络中，需要允许 lolPing 通过防火墙；在不同网络中，双方都需要打开“允许互联网连接”。",
    ping_names: PingNameStrings { danger: "危险", push: "推进", omw: "正在赶来", allin: "全力进攻", assist: "请求协助", needvision: "需要视野", missing: "敌人消失", enemyvision: "敌方视野", generic: "普通信号", bait: "诱饵", visioncleared: "视野已清除" },
    limitations: &["无法在以管理员身份运行的窗口（例如任务管理器）上打开轮盘。", "独占全屏的游戏会盖住信号层。", "只共享单个窗口时看不到信号，请共享整个屏幕。", "lolPing 开启时，Ctrl + 拖动（默认的表情键）会打开表情轮盘，而不是复制文件或文字。可在“设置 → 表情”中更换表情键或将其关闭。"],
    emote_key_clash: "信号触发键和表情键不能是同一个键，表情键也不能是暂停快捷键的按键。",
    problem_emote_key_clash: "表情键与 lolPing 的另一个按键相同，已被关闭。",
    toast_on: "信号已开启",
    toast_off: "信号已关闭",
    toast_helper_stopped: "信号助手已停止",
    toast_helper_stopped_body: "右键单击托盘图标以重启",
    tray_tip_failed: "lolPing：输入助手已停止",
    tray_enabled: "已开启",
    tray_restart: "重启输入助手",
    tray_settings: "设置…",
    tray_quit: "退出 lolPing",
    tray_room: "房间",
    tray_room_join_clipboard: "从剪贴板加入",
    tray_room_create: "创建房间",
    tray_room_settings: "房间设置…",
    tray_room_copy: "复制房间码",
    tray_room_mute: "房间静音",
    tray_room_leave: "离开房间",
    toast_room_joined_body: "把房间码分享给朋友",
    toast_room_full: "房间已满",
    toast_room_full_body: "一个房间最多 8 人",
    toast_no_code: "剪贴板中没有房间码",
    toast_no_code_body: "请先复制形如 PING-XXXXX-XXXXX 的房间码",
    update_now: "更新",
    whats_new: "更新内容",
    check_for_updates: "检查更新",
    checking: "正在检查…",
    up_to_date: "lolPing 已是最新版本",
    installing_update: "正在安装更新…",
    update_toast_title: "有可用更新",
    update_error_toast_title: "更新出错",
    restarting_to_update: "正在重启以更新…",
    update_mac_steps: "lolPing 会下载更新并打开它。请把 lolPing 拖到“应用程序”中，选择“替换”，然后打开它并再次允许辅助功能。",
    update_mac_dialog: "请把 lolPing 拖到“应用程序”中并选择“替换”，然后打开它并再次允许辅助功能。lolPing 现在将退出。",
    update_err_download: "下载失败，请重试。",
    update_err_damaged: "下载的文件已损坏，请重试。",
    update_err_install: "无法安装更新，请重试。",
    update_reason_unknown: "没有详细信息",
    retry: "重试",
    auto_update_check: "自动检查更新",
    auto_update_check_desc: "每隔几小时向 GitHub 检查一次新版本。在你点击“更新”之前不会下载任何内容。",
    tray_check_for_updates: "检查更新",
    tray_installing: "正在安装更新…",
};

pub fn strings(lang: Lang) -> &'static Strings {
    if lang == LANG_ZH_CN {
        &ZH
    } else {
        &EN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_count_matches_reference() {
        // The reference table has 215 keys and 37 of them are methods here; 11 describe macOS-only behaviour that is gone.
        assert_eq!(Strings::FIELD_COUNT + 37, 215 - 11);
    }

    #[test]
    fn tables_differ_between_languages() {
        let en = strings("en");
        let zh = strings("zh-CN");
        assert_ne!(en.lang, zh.lang);
        assert_ne!(en.on, zh.on);
        assert_ne!(en.page_title, zh.page_title);
        assert_ne!(en.limitations[0], zh.limitations[0]);
        assert_ne!(en.status_hint("Ctrl", "F8", true), zh.status_hint("Ctrl", "F8", true));
        assert_ne!(en.room_limit_value(0), zh.room_limit_value(0));
        assert_ne!(en.ping_names.danger, zh.ping_names.danger);
    }

    #[test]
    fn strings_returns_one_object_per_language() {
        assert!(std::ptr::eq(strings("en"), strings(LANG_EN)));
        assert!(std::ptr::eq(strings("zh-CN"), strings(LANG_ZH_CN)));
        assert!(!std::ptr::eq(strings("en"), strings("zh-CN")));
    }

    #[test]
    fn resolve_lang_follows_locale() {
        assert_eq!(resolve_lang(LanguagePref::Auto, "zh-CN"), LANG_ZH_CN);
        assert_eq!(resolve_lang(LanguagePref::Auto, "zh-Hans"), LANG_ZH_CN);
        assert_eq!(resolve_lang(LanguagePref::Auto, "en-GB"), LANG_EN);
        assert_eq!(resolve_lang(LanguagePref::Auto, ""), LANG_EN);
        assert_eq!(resolve_lang(LanguagePref::En, "zh-CN"), LANG_EN);
        assert_eq!(resolve_lang(LanguagePref::ZhCn, "en-US"), LANG_ZH_CN);
    }
}
