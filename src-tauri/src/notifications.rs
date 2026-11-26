use std::{
    collections::HashMap,
    ffi::{c_char, c_int, c_void, CStr, CString},
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::FILETIME, System::SystemInformation::GetSystemTimeAsFileTime,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct AppNotification {
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    capped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BadgeSignal {
    Clear,
    Unread(AppNotification),
}

impl BadgeSignal {
    pub fn notification(self) -> Option<AppNotification> {
        match self {
            Self::Clear => None,
            Self::Unread(notification) => Some(notification),
        }
    }

    pub fn from_description(description: &str) -> Option<Self> {
        let words: Vec<_> = description
            .split(|c: char| !c.is_alphanumeric() && c != '+')
            .filter(|word| !word.is_empty())
            .map(str::to_lowercase)
            .collect();
        let is_subject = |word: &str| {
            matches!(
                word,
                "notification" | "notifications" | "mention" | "mentions" | "message" | "messages"
            )
        };
        for (index, word) in words.iter().enumerate() {
            if !is_subject(word) {
                continue;
            }
            let preceding = &words[..index];
            let modifier = preceding.last().map(String::as_str);
            let unread = matches!(modifier, Some("unread" | "new"));
            let count = if unread {
                preceding.iter().rev().nth(1)
            } else {
                preceding.last()
            };
            if let Some(count) = count {
                if count == "no" {
                    return Some(Self::Clear);
                }
                let digits = count.trim_end_matches('+');
                if let Ok(number) = digits.parse::<u32>() {
                    return Some(
                        AppNotification::numbered(number, digits != count)
                            .map_or(Self::Clear, Self::Unread),
                    );
                }
            }
            if unread {
                return Some(Self::Unread(AppNotification::dot()));
            }
        }
        None
    }
}

impl AppNotification {
    fn dot() -> Self {
        Self {
            count: None,
            capped: false,
        }
    }

    fn numbered(count: u32, capped: bool) -> Option<Self> {
        (count > 0).then_some(Self {
            count: Some(count),
            capped,
        })
    }

    fn from_badge(payload: &str) -> Option<Self> {
        let badge = payload.split_once("<badge")?.1.split_once('>')?.0;
        let value = badge
            .split_once("value")?
            .1
            .trim_start()
            .strip_prefix('=')?
            .trim_start();
        let quote = value
            .chars()
            .next()
            .filter(|quote| matches!(quote, '"' | '\''))?;
        let value = value.get(1..)?.split_once(quote)?.0;
        if let Ok(count) = value.parse::<u32>() {
            return Self::numbered(count, false);
        }
        matches!(value, "newMessage" | "alert" | "attention").then(Self::dot)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MessagingApp {
    Teams,
    PersonalTeams,
    ClassicTeams,
    Discord,
    DiscordPtb,
    DiscordCanary,
}

impl MessagingApp {
    pub fn from_identity(identity: &str) -> Option<Self> {
        match identity.split('!').next()? {
            "msteams_8wekyb3d8bbwe" => Some(Self::Teams),
            "microsoftteams_8wekyb3d8bbwe" => Some(Self::PersonalTeams),
            "com.squirrel.teams.teams" => Some(Self::ClassicTeams),
            "com.squirrel.discord.discord" => Some(Self::Discord),
            "com.squirrel.discordptb.discordptb" => Some(Self::DiscordPtb),
            "com.squirrel.discordcanary.discordcanary" => Some(Self::DiscordCanary),
            _ => None,
        }
    }

    pub fn from_executable(exe: &str) -> Option<Self> {
        match exe.to_ascii_lowercase().as_str() {
            "ms-teams.exe" => Some(Self::Teams),
            "msteams.exe" => Some(Self::PersonalTeams),
            "teams.exe" => Some(Self::ClassicTeams),
            "discord.exe" => Some(Self::Discord),
            "discordptb.exe" => Some(Self::DiscordPtb),
            "discordcanary.exe" => Some(Self::DiscordCanary),
            _ => None,
        }
    }
}

#[derive(Clone, Default)]
pub struct Notifications {
    pub badges: HashMap<String, AppNotification>,
    app_badges: HashMap<MessagingApp, AppNotification>,
    toasts: HashMap<MessagingApp, i64>,
}

impl Notifications {
    pub fn messaging_badge(&self, app: MessagingApp) -> Option<AppNotification> {
        self.app_badges.get(&app).copied()
    }

    pub fn toast_badge(&self, app: MessagingApp, seen_through: i64) -> Option<AppNotification> {
        self.toasts
            .get(&app)
            .filter(|&&arrival| arrival > seen_through)
            .map(|_| AppNotification::dot())
    }
}

// Toasts have a separate lifetime from app unread counts. Read their metadata
// for recent activity and reserve badge payloads for app-provided counts.
#[link(name = "winsqlite3")]
extern "C" {
    fn sqlite3_open_v2(
        path: *const c_char,
        db: *mut *mut c_void,
        flags: c_int,
        vfs: *const c_char,
    ) -> c_int;
    fn sqlite3_close(db: *mut c_void) -> c_int;
    fn sqlite3_prepare_v2(
        db: *mut c_void,
        sql: *const c_char,
        len: c_int,
        stmt: *mut *mut c_void,
        tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_finalize(stmt: *mut c_void) -> c_int;
    fn sqlite3_bind_int64(stmt: *mut c_void, index: c_int, value: i64) -> c_int;
    fn sqlite3_step(stmt: *mut c_void) -> c_int;
    fn sqlite3_column_int(stmt: *mut c_void, column: c_int) -> c_int;
    fn sqlite3_column_int64(stmt: *mut c_void, column: c_int) -> i64;
    fn sqlite3_column_text(stmt: *mut c_void, column: c_int) -> *const c_char;
}

struct Database(*mut c_void);

impl Drop for Database {
    fn drop(&mut self) {
        unsafe { sqlite3_close(self.0) };
    }
}

struct Statement(*mut c_void);

impl Drop for Statement {
    fn drop(&mut self) {
        unsafe { sqlite3_finalize(self.0) };
    }
}

impl Statement {
    fn text(&self, column: c_int) -> Option<&str> {
        let value = unsafe { sqlite3_column_text(self.0, column) };
        if value.is_null() {
            return None;
        }
        unsafe { CStr::from_ptr(value) }.to_str().ok()
    }
}

const READ_ONLY: c_int = 1;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;
const NOTIFICATIONS_SQL: &CStr = c"
    SELECT h.PrimaryId,
           CASE WHEN n.Type = 'badge' COLLATE NOCASE THEN CAST(n.Payload AS TEXT) END,
           (n.ExpiryTime = 0 OR n.ExpiryTime > ?1), n.ArrivalTime,
           n.Type = 'badge' COLLATE NOCASE
    FROM Notification n JOIN NotificationHandler h ON h.RecordId = n.HandlerId
    WHERE n.Type = 'badge' COLLATE NOCASE
       OR (n.Type = 'toast' COLLATE NOCASE AND n.ArrivalTime > ?2)
    ORDER BY n.ArrivalTime DESC, n.Id DESC
";

pub(crate) fn current_filetime() -> i64 {
    let mut time = FILETIME::default();
    unsafe { GetSystemTimeAsFileTime(&mut time) };
    (i64::from(time.dwHighDateTime) << 32) | i64::from(time.dwLowDateTime)
}

pub fn read(since: i64) -> Option<Notifications> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    let path = Path::new(&local).join(r"Microsoft\Windows\Notifications\wpndatabase.db");
    let filename = CString::new(path.to_str()?).ok()?;
    read_database(&filename, current_filetime(), since)
}

fn read_database(filename: &CStr, filetime: i64, since: i64) -> Option<Notifications> {
    let mut db = Database(null_mut());
    if unsafe { sqlite3_open_v2(filename.as_ptr(), &mut db.0, READ_ONLY, null()) } != 0 {
        return None;
    }
    read_notifications(&db, filetime, since)
}

fn read_notifications(db: &Database, filetime: i64, since: i64) -> Option<Notifications> {
    let mut stmt = Statement(null_mut());
    unsafe {
        if sqlite3_prepare_v2(
            db.0,
            NOTIFICATIONS_SQL.as_ptr(),
            -1,
            &mut stmt.0,
            null_mut(),
        ) != 0
            || sqlite3_bind_int64(stmt.0, 1, filetime) != 0
            || sqlite3_bind_int64(stmt.0, 2, since) != 0
        {
            return None;
        }
        let mut entries = Notifications::default();
        let mut seen_badges = std::collections::HashSet::new();
        let mut seen_app_badges = std::collections::HashSet::new();
        loop {
            match sqlite3_step(stmt.0) {
                SQLITE_ROW => {
                    let Some(identity) = stmt.text(0).filter(|value| !value.is_empty()) else {
                        continue;
                    };
                    let identity = identity.to_lowercase();
                    let app = MessagingApp::from_identity(&identity);
                    let active = sqlite3_column_int(stmt.0, 2) == 1;
                    if sqlite3_column_int(stmt.0, 4) == 1 {
                        if !seen_badges.insert(identity.clone()) {
                            continue;
                        }
                        let newest_for_app = app.filter(|app| seen_app_badges.insert(*app));
                        if active {
                            if let Some(notification) =
                                stmt.text(1).and_then(AppNotification::from_badge)
                            {
                                entries.badges.insert(identity, notification);
                                if let Some(app) = newest_for_app {
                                    entries.app_badges.insert(app, notification);
                                }
                            }
                        }
                    } else if active {
                        if let Some(app) = app {
                            entries
                                .toasts
                                .entry(app)
                                .or_insert_with(|| sqlite3_column_int64(stmt.0, 3));
                        }
                    }
                }
                SQLITE_DONE => return Some(entries),
                _ => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_unread_counts_from_running_window_counts() {
        assert_eq!(
            BadgeSignal::from_description("Discord, 1 running window, 99+ unread mentions"),
            AppNotification::numbered(99, true).map(BadgeSignal::Unread)
        );
        assert_eq!(
            BadgeSignal::from_description("Unread messages"),
            Some(BadgeSignal::Unread(AppNotification::dot()))
        );
        assert_eq!(
            BadgeSignal::from_description("No new notifications"),
            Some(BadgeSignal::Clear)
        );
        assert_eq!(BadgeSignal::from_description("2 running windows"), None);
        assert_eq!(
            BadgeSignal::from_description("Notifications settings"),
            None
        );
        assert_eq!(
            BadgeSignal::from_description("0 notifications"),
            Some(BadgeSignal::Clear)
        );
    }

    #[test]
    fn reads_new_messaging_toasts_and_clears_expired_or_zero_badges() {
        let mut db = Database(null_mut());
        assert_eq!(
            unsafe { sqlite3_open_v2(c":memory:".as_ptr(), &mut db.0, 6, null()) },
            0
        );
        let run = |sql: &CStr| {
            let mut stmt = Statement(null_mut());
            assert_eq!(
                unsafe { sqlite3_prepare_v2(db.0, sql.as_ptr(), -1, &mut stmt.0, null_mut()) },
                0
            );
            assert_eq!(unsafe { sqlite3_step(stmt.0) }, SQLITE_DONE);
        };
        run(c"CREATE TABLE NotificationHandler (RecordId INTEGER, PrimaryId TEXT)");
        run(c"CREATE TABLE Notification (Id INTEGER, HandlerId INTEGER, Type TEXT, Payload BLOB, ArrivalTime INTEGER, ExpiryTime INTEGER)");
        run(c"INSERT INTO NotificationHandler VALUES (1, 'com.squirrel.Discord.Discord'), (2, 'App.B'), (3, 'MSTeams_8wekyb3d8bbwe!MSTeams'), (4, 'com.squirrel.DiscordCanary.DiscordCanary')");
        run(c"INSERT INTO Notification VALUES
            (1, 1, 'toast', '<toast>Message</toast>', 9, 16),
            (2, 1, 'toast', '<toast>Message</toast>', 8, 15),
            (3, 1, 'toast', NULL, 1, 10),
            (4, 2, 'toast', NULL, 8, 0),
            (5, 2, 'badge', '<badge value=\"8\" />', 9, 0),
            (6, 2, 'badge', '<badge value=\"4\" />', 10, 0),
            (7, 3, 'toast', NULL, 7, 0),
            (8, 4, 'toast', NULL, 9, 10)");
        let snapshot = read_notifications(&db, 10, 7).unwrap();
        assert!(!snapshot.badges.contains_key("com.squirrel.discord.discord"));
        assert_eq!(
            snapshot.badges["app.b"],
            AppNotification::numbered(4, false).unwrap()
        );
        assert_eq!(
            snapshot.toast_badge(MessagingApp::Discord, 8),
            Some(AppNotification::dot())
        );
        assert_eq!(snapshot.toast_badge(MessagingApp::Discord, 9), None);
        assert_eq!(snapshot.toast_badge(MessagingApp::Teams, 0), None);
        assert_eq!(snapshot.toast_badge(MessagingApp::DiscordCanary, 0), None);
        run(c"INSERT INTO Notification VALUES (9, 3, 'toast', NULL, 10, 15)");
        assert_eq!(
            read_notifications(&db, 10, 7)
                .unwrap()
                .toast_badge(MessagingApp::Teams, 0),
            Some(AppNotification::dot())
        );
        run(c"DELETE FROM Notification WHERE Id = 9");
        assert_eq!(
            read_notifications(&db, 10, 7)
                .unwrap()
                .toast_badge(MessagingApp::Teams, 0),
            None
        );
        run(c"UPDATE Notification SET ExpiryTime = 10 WHERE Id = 6");
        assert!(read_notifications(&db, 10, 7).unwrap().badges.is_empty());
        run(c"UPDATE Notification SET ExpiryTime = 0, Payload = '<badge value=\"0\" />' WHERE Id = 6");
        assert!(read_notifications(&db, 10, 7).unwrap().badges.is_empty());
        run(c"DELETE FROM Notification WHERE Type = 'badge'");
        assert!(read_notifications(&db, 10, 7).unwrap().badges.is_empty());
    }
}
