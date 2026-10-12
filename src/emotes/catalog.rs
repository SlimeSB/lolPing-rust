#[derive(Clone, Copy, Debug)]
pub struct CatalogEmote {
    pub slug: &'static str,
    pub league_id: u32,
    pub name: &'static str,
    pub name_zh: &'static str,
    pub has_sound: bool,
}

pub const EMOTES: &[CatalogEmote] = &[
    CatalogEmote { slug: "facepalm", league_id: 4341, name: "Facepalm", name_zh: "捂脸", has_sound: true },
    CatalogEmote { slug: "unworthy", league_id: 3236, name: "Unworthy", name_zh: "弱爆", has_sound: true },
    CatalogEmote { slug: "unworthy_cn", league_id: 3239, name: "Unworthy (Chinese text)", name_zh: "弱爆（文字版）", has_sound: true },
    CatalogEmote { slug: "poro_snax", league_id: 1445, name: "Poro Snax", name_zh: "魄罗佳肴", has_sound: true },
    CatalogEmote { slug: "sad_kitten", league_id: 1457, name: "Sad Kitten", name_zh: "悲伤猫咪", has_sound: true },
    CatalogEmote { slug: "cheeky_poro", league_id: 1499, name: "Cheeky Poro", name_zh: "调皮魄罗", has_sound: true },
    CatalogEmote { slug: "angry_kitty", league_id: 1501, name: "Angry Kitty", name_zh: "我生气了！", has_sound: true },
    CatalogEmote { slug: "gg_heart", league_id: 3124, name: "GG <3", name_zh: "精彩的对局 <3", has_sound: true },
    CatalogEmote { slug: "bee_happy", league_id: 3153, name: "Bee Happy", name_zh: "小蜜蜂 高兴", has_sound: true },
    CatalogEmote { slug: "bee_mad", league_id: 3154, name: "Bee Mad", name_zh: "小蜜蜂 疯狂", has_sound: true },
    CatalogEmote { slug: "bee_sad", league_id: 3155, name: "Bee Sad", name_zh: "小蜜蜂 悲伤", has_sound: true },
    CatalogEmote { slug: "snoozy_poro", league_id: 3173, name: "Snoozy Poro", name_zh: "打盹魄罗", has_sound: true },
    CatalogEmote { slug: "wahaha", league_id: 3177, name: "Wahaha", name_zh: "笑哭我了", has_sound: true },
    CatalogEmote { slug: "omg_love_it", league_id: 3207, name: "OMG I love it", name_zh: "我的天，爱死它了", has_sound: true },
    CatalogEmote { slug: "poro_ride", league_id: 3214, name: "Poro Ride", name_zh: "溜了溜了", has_sound: true },
    CatalogEmote { slug: "dont_make_me_laugh", league_id: 3358, name: "Don't Make Me Laugh", name_zh: "xswl", has_sound: true },
    CatalogEmote { slug: "woah_dizzy", league_id: 4334, name: "Woah…Dizzy", name_zh: "晕晕", has_sound: true },
    CatalogEmote { slug: "haha_hilarious", league_id: 4443, name: "Haha HILARIOUS", name_zh: "哈哈笑死", has_sound: true },
    CatalogEmote { slug: "gasp", league_id: 4670, name: "Gasp!", name_zh: "倒吸一口凉气！", has_sound: true },
    CatalogEmote { slug: "super_approved", league_id: 4860, name: "Super-Approved", name_zh: "超赞的", has_sound: true },
    CatalogEmote { slug: "pain", league_id: 4948, name: "PaiN…", name_zh: "强颜欢笑", has_sound: true },
    CatalogEmote { slug: "laughing_out_loud", league_id: 4952, name: "Laughing Out LOUD", name_zh: "哈哈哈", has_sound: true },
    CatalogEmote { slug: "hats_off", league_id: 3156, name: "Hat's Off", name_zh: "脱帽致意", has_sound: true },
    CatalogEmote { slug: "call_me", league_id: 3338, name: "Call Me!", name_zh: "双击666！", has_sound: true },
    CatalogEmote { slug: "think_about_it", league_id: 3339, name: "Think About It", name_zh: "脑子是个好东西", has_sound: true },
    CatalogEmote { slug: "unimpressed", league_id: 3414, name: "Animated Unimpressed!", name_zh: "凡人！动态表情", has_sound: true },
    CatalogEmote { slug: "did_you_just", league_id: 3448, name: "Animated Did You Just?", name_zh: "动态 我太难了！", has_sound: true },
    CatalogEmote { slug: "nice_try", league_id: 1030, name: "Nice Try", name_zh: "棒棒哒", has_sound: true },
    CatalogEmote { slug: "nice", league_id: 1459, name: "Nice", name_zh: "干得漂亮", has_sound: true },
    CatalogEmote { slug: "despair", league_id: 1467, name: "Despair", name_zh: "呜呜呜---", has_sound: true },
    CatalogEmote { slug: "does_not_compute", league_id: 1468, name: "Does Not Compute", name_zh: "无法求解", has_sound: true },
    CatalogEmote { slug: "come_at_me", league_id: 1480, name: "Come at Me", name_zh: "放马过来", has_sound: true },
    CatalogEmote { slug: "how_could_you", league_id: 1487, name: "How Could You", name_zh: "你怎能这样", has_sound: true },
    CatalogEmote { slug: "scout_approved", league_id: 1492, name: "Scout-Approved", name_zh: "提莫说很好", has_sound: true },
    CatalogEmote { slug: "maybe_next_time", league_id: 3141, name: "Maybe Next Time", name_zh: "也许下次", has_sound: true },
    CatalogEmote { slug: "okay", league_id: 3159, name: "Okay", name_zh: "OK", has_sound: true },
    CatalogEmote { slug: "good_job_buddy", league_id: 3166, name: "Good Job, Buddy!", name_zh: "干得好，伙计！", has_sound: true },
    CatalogEmote { slug: "teamwork", league_id: 3167, name: "Teamwork!", name_zh: "团队协作！", has_sound: true },
    CatalogEmote { slug: "i_will_destroy_you", league_id: 3215, name: "I will destroy you", name_zh: "你瞅啥？", has_sound: true },
    CatalogEmote { slug: "not_today", league_id: 3581, name: "Not Today!", name_zh: "没门！", has_sound: true },
    CatalogEmote { slug: "oh", league_id: 3709, name: "Oh...", name_zh: "大受震撼", has_sound: true },
    CatalogEmote { slug: "come_again", league_id: 3962, name: "Come again?", name_zh: "一脸问号", has_sound: true },
    CatalogEmote { slug: "magpie_oopsie", league_id: 4091, name: "Magpie Oopsie", name_zh: "雀实有问题", has_sound: true },
    CatalogEmote { slug: "such_foolishness", league_id: 4730, name: "Such Foolishness", name_zh: "如此愚蠢", has_sound: true },
];

pub const DEFAULT_EMOTE_WHEEL: [&str; 8] = [
    "unworthy_cn",
    "poro_snax",
    "sad_kitten",
    "cheeky_poro",
    "angry_kitty",
    "gg_heart",
    "bee_happy",
    "bee_mad",
];

pub const DEFAULT_CLICK_EMOTE: &str = "facepalm";

pub fn by_slug(slug: &str) -> Option<&'static CatalogEmote> {
    EMOTES.iter().find(|e| e.slug == slug)
}
