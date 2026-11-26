use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DockEdge {
    #[default]
    Bottom,
    Top,
    Left,
    Right,
}

impl DockEdge {
    pub fn vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Alignment {
    Start,
    #[default]
    Center,
    End,
}

impl Alignment {
    pub fn offset(self, space: i32) -> i32 {
        match self {
            Self::Start => 0,
            Self::Center => space / 2,
            Self::End => space,
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Material {
    #[serde(alias = "glass")]
    Tinted,
    Midnight,
    Clear,
    Spotify,
}

#[derive(Clone, Copy)]
struct TintColor([u8; 3]);

impl Default for TintColor {
    fn default() -> Self {
        Self([200, 182, 255])
    }
}

impl Serialize for TintColor {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let [red, green, blue] = self.0;
        serializer.serialize_str(&format!("#{red:02x}{green:02x}{blue:02x}"))
    }
}

impl<'de> Deserialize<'de> for TintColor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(value
            .as_str()
            .and_then(|value| value.strip_prefix('#'))
            .filter(|hex| hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .map_or_else(Self::default, |rgb| {
                let [_, red, green, blue] = rgb.to_be_bytes();
                Self([red, green, blue])
            }))
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Accent {
    Red,
    #[serde(alias = "peach")]
    Orange,
    Yellow,
    #[serde(alias = "mint")]
    Green,
    #[serde(alias = "sky")]
    Blue,
    #[serde(alias = "lilac")]
    Purple,
    Pink,
    Dark,
    Light,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum AppTheme {
    Light,
    Dark,
    #[default]
    #[serde(other)]
    System,
}

#[derive(Clone, Copy, Serialize)]
#[serde(transparent)]
struct AppAppearancePercent(u8);

impl Default for AppAppearancePercent {
    fn default() -> Self {
        Self(100)
    }
}

impl<'de> Deserialize<'de> for AppAppearancePercent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(value.as_f64().map_or_else(Self::default, |amount| {
            Self(amount.round().clamp(0.0, 100.0) as u8)
        }))
    }
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AppAppearance {
    theme: AppTheme,
    tint_color: TintColor,
    blur_amount: AppAppearancePercent,
    tint_amount: AppAppearancePercent,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum RevealAnimation {
    Easing,
    Bounce,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum AppIconShape {
    #[default]
    Original,
    Rounded,
    Circle,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum AppIconBackground {
    #[default]
    Transparent,
    Tinted,
    Black,
    White,
    Custom,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum AppIconClickEffect {
    #[default]
    None,
    Press,
    Stars,
    Squish,
    Bounce,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum NotificationBadges {
    Dot,
    None,
    Count,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PreviewDisplay {
    Window,
    Dock,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub edge: DockEdge,
    pub alignment: Alignment,
    pub full_span: bool,
    pub icon_size: u32,
    pub spacing: u32,
    pub padding: u32,
    pub offset: u32,
    pub zoom: f64,
    pub falloff: f64,
    pub duration: u32,
    pub magnify: bool,
    pub hide_settings_icon: bool,
    pub auto_hide: bool,
    pub hide_in_fullscreen: bool,
    pub replace_taskbar: bool,
    pub replace_windows_start: bool,
    pub reveal_duration: u32,
    reveal_animation: RevealAnimation,
    bounce_amount: u32,
    names: bool,
    previews: bool,
    pub(crate) preview_display: PreviewDisplay,
    indicators: bool,
    pub(crate) notification_badges: NotificationBadges,
    pub(crate) material: Material,
    spotify_frosted: bool,
    pub(crate) dock_snow: bool,
    tint_color: TintColor,
    accent: Accent,
    app_icon_shape: AppIconShape,
    app_icon_background: AppIconBackground,
    app_icon_color: TintColor,
    app_icon_click_effect: AppIconClickEffect,
    app_icon_click_speed: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            edge: DockEdge::Bottom,
            alignment: Alignment::Center,
            full_span: false,
            icon_size: 46,
            spacing: 10,
            padding: 8,
            offset: 12,
            zoom: 1.55,
            falloff: 2.1,
            duration: 420,
            magnify: true,
            hide_settings_icon: false,
            auto_hide: false,
            hide_in_fullscreen: true,
            replace_taskbar: true,
            replace_windows_start: false,
            reveal_duration: 260,
            reveal_animation: RevealAnimation::Easing,
            bounce_amount: 35,
            names: true,
            previews: true,
            preview_display: PreviewDisplay::Window,
            indicators: true,
            notification_badges: NotificationBadges::Dot,
            material: Material::Tinted,
            spotify_frosted: true,
            dock_snow: false,
            tint_color: TintColor::default(),
            accent: Accent::Purple,
            app_icon_shape: AppIconShape::default(),
            app_icon_background: AppIconBackground::default(),
            app_icon_color: TintColor([48, 52, 62]),
            app_icon_click_effect: AppIconClickEffect::default(),
            app_icon_click_speed: 100,
        }
    }
}

impl Settings {
    pub(crate) fn legacy_app_appearance(self) -> AppAppearance {
        let (theme, tint) = match self.accent {
            Accent::Red => (AppTheme::System, [239, 102, 108]),
            Accent::Orange => (AppTheme::System, [248, 155, 94]),
            Accent::Yellow => (AppTheme::System, [242, 204, 96]),
            Accent::Green => (AppTheme::System, [121, 220, 176]),
            Accent::Blue => (AppTheme::System, [131, 186, 255]),
            Accent::Purple => return AppAppearance::default(),
            Accent::Pink => (AppTheme::System, [237, 152, 192]),
            Accent::Dark => (AppTheme::Dark, [52, 54, 61]),
            Accent::Light => (AppTheme::Light, [232, 233, 237]),
        };
        AppAppearance {
            theme,
            tint_color: TintColor(tint),
            ..AppAppearance::default()
        }
    }

    pub(crate) fn normalized(mut self) -> Self {
        self.icon_size = self.icon_size.clamp(20, 80);
        self.spacing = self.spacing.min(32);
        self.padding = self.padding.min(24);
        self.offset = self.offset.min(60);
        self.zoom = self.zoom.clamp(1.0, 3.0);
        self.falloff = self.falloff.clamp(0.5, 5.0);
        self.duration = self.duration.clamp(60, 1000);
        self.reveal_duration = self.reveal_duration.clamp(60, 1000);
        self.bounce_amount = self.bounce_amount.min(100);
        self.app_icon_click_speed = self.app_icon_click_speed.clamp(50, 200);
        self
    }
}

pub fn save_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("tmp");
    fs::create_dir_all(path.parent().expect("configuration directory"))
        .and_then(|()| fs::write(&temporary, data))
        .and_then(|()| fs::rename(temporary, path))
        .map_err(|error| format!("Couldn’t save settings. {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_amounts_default_for_older_preferences_and_stay_in_range() {
        let mut stored = serde_json::json!({"theme": "dark", "tintColor": "#52a1cf"});
        let older: AppAppearance = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(older.blur_amount.0, 100);
        assert_eq!(older.tint_amount.0, 100);

        stored["blurAmount"] = serde_json::json!(67);
        let older: AppAppearance = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(older.blur_amount.0, 67);
        assert_eq!(older.tint_amount.0, 100);

        for (input, expected) in [
            (serde_json::json!(-3), 0),
            (serde_json::json!(0), 0),
            (serde_json::json!(74.6), 75),
            (serde_json::json!(130), 100),
            (serde_json::json!("invalid"), 100),
        ] {
            stored["blurAmount"] = input.clone();
            stored["tintAmount"] = input;
            let appearance: AppAppearance = serde_json::from_value(stored.clone()).unwrap();
            assert_eq!(
                serde_json::to_value(appearance).unwrap(),
                serde_json::json!({
                    "theme": "dark",
                    "tintColor": "#52a1cf",
                    "blurAmount": expected,
                    "tintAmount": expected,
                })
            );
        }
    }

    #[test]
    fn spotify_material_defaults_to_frosted_and_saves_the_solid_finish() {
        let mut settings: Settings = serde_json::from_str(r#"{"material":"spotify"}"#).unwrap();
        assert!(matches!(settings.material, Material::Spotify));
        assert!(settings.spotify_frosted);
        settings.spotify_frosted = false;
        let saved = serde_json::to_value(settings).unwrap();
        assert_eq!(saved["material"], "spotify");
        assert_eq!(saved["spotifyFrosted"], false);
    }

    #[test]
    fn upgrades_old_glass_and_keeps_other_preferences() {
        assert!(matches!(Settings::default().material, Material::Tinted));
        let old: Settings = serde_json::from_slice(br#"{"material":"glass","offset":36}"#).unwrap();
        assert!(matches!(old.material, Material::Tinted));
        assert!(matches!(old.notification_badges, NotificationBadges::Dot));
        assert_eq!(old.preview_display, PreviewDisplay::Window);
        assert_eq!(old.offset, 36);
        let custom: Settings = serde_json::from_slice(
            br##"{"material":"glass","tintColor":"#48A0D5","accent":"mint"}"##,
        )
        .unwrap();
        let saved = serde_json::to_value(custom).unwrap();
        assert_eq!(saved["material"], "tinted");
        assert_eq!(saved["tintColor"], "#48a0d5");
        assert_eq!(saved["accent"], "green");
        let invalid: Settings =
            serde_json::from_slice(br#"{"material":"clear","tintColor":42,"offset":18}"#).unwrap();
        assert!(matches!(invalid.material, Material::Clear));
        assert_eq!(invalid.offset, 18);
        assert_eq!(
            serde_json::to_value(invalid).unwrap()["tintColor"],
            "#c8b6ff"
        );
    }

    #[test]
    fn saves_the_dock_monitor_preview_preference() {
        let saved: Settings = serde_json::from_slice(br#"{"previewDisplay":"dock"}"#).unwrap();
        assert_eq!(saved.preview_display, PreviewDisplay::Dock);
        assert_eq!(
            serde_json::to_value(saved).unwrap()["previewDisplay"],
            "dock"
        );
    }

    #[test]
    fn click_animation_speed_defaults_to_normal_and_stays_in_range() {
        let prior: Settings = serde_json::from_str(r#"{"appIconClickEffect":"stars"}"#).unwrap();
        assert_eq!(prior.app_icon_click_speed, 100);
        for (input, expected) in [(0, 50), (175, 175), (300, 200)] {
            let settings: Settings =
                serde_json::from_value(serde_json::json!({ "appIconClickSpeed": input })).unwrap();
            assert_eq!(
                serde_json::to_value(settings.normalized()).unwrap()["appIconClickSpeed"],
                expected
            );
        }
    }
}
