use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;
use zbus::{fdo, interface, object_server::SignalEmitter};

use crate::attachments;
use crate::db::{self, Db, Status};
use crate::health::Health;
use crate::login::new_uuid;
use crate::notify::Notifier;
use crate::store;

pub const NAME: &str = "org.sailbubbles.Daemon";
pub const PATH: &str = "/org/sailbubbles/Daemon";

pub enum Outgoing {
    Text { id: String, chat_id: String, text: String, reply_to: Option<String> },
    File { id: String, chat_id: String, path: PathBuf, name: String, mime: String, reply_to: Option<String> },
    /// The chat was read here; tells our other devices, and the other side
    /// too when read receipts are on.
    Read { chat_id: String },
    React { chat_id: String, message_id: String, kind: String, enable: bool },
    Edit { chat_id: String, message_id: String, text: String },
    Unsend { chat_id: String, message_id: String },
    Typing { chat_id: String, typing: bool },
    // Deletes and unread marks go to our own other devices only, never to the
    // other side, as on Apple devices.
    DeleteMessage { message_id: String },
    DeleteChat { participants: Vec<String>, guid: Option<String> },
    MarkUnread { message_id: String },
    Rename { chat_id: String, name: String },
    /// Sends the group's new member list; `change` is the event to record once sent.
    Participants { chat_id: String, participants: Vec<String>, change: (db::Event, String) },
    /// A 570x570 PNG made by the app, or None to remove the group photo.
    Icon { chat_id: String, png: Option<PathBuf> },
}

// Apple's limits for changing a sent message.
const EDIT_WINDOW_MS: i64 = 15 * 60 * 1000;
const UNSEND_WINDOW_MS: i64 = 2 * 60 * 1000;

pub const REACTIONS: &[&str] = &["heart", "like", "dislike", "laugh", "emphasize", "question"];

// Send only stores and queues; the task that owns rustpush does the sending,
// so D-Bus calls return at once and never wait on Apple.
pub struct Api {
    pub db: Arc<Mutex<Db>>,
    /// Our first handle, lowercased, which new chats are started from.
    pub me: String,
    /// All our handles, lowercased; a chat can be with any of them.
    pub mine: Vec<String>,
    pub outbox: mpsc::Sender<Outgoing>,
    pub notifier: Arc<Notifier>,
    pub health: Arc<Mutex<Health>>,
    pub downloads: mpsc::Sender<(String, u32)>,
    pub attachments_root: PathBuf,
    pub home: PathBuf,
    pub state_dir: PathBuf,
}

/// Known settings and their defaults.
pub const SETTINGS: &[(&str, bool)] = &[("previews", true), ("readReceipts", false), ("filterUnknown", false),
                                         ("unarchiveOnNewMessage", false)];

pub fn setting(db: &Db, key: &str) -> bool {
    let default = SETTINGS.iter().find(|(k, _)| *k == key).map_or(false, |(_, d)| *d);
    db.setting(key, default).unwrap_or(default)
}

/// The chat's title (with contact names), unread count, and whether it is a
/// one-to-one chat with someone not in the contacts.
pub fn chat_summary(db: &Db, chat_id: &str, mine: &[String]) -> rusqlite::Result<Option<(String, u32, bool)>> {
    let names = db.names()?;
    Ok(db.chats()?.into_iter().find(|c| c.id == chat_id).map(|c| {
        let unknown = !known(&c, mine, &names);
        (title(&c.participants, c.name, mine, &names), c.unread, unknown)
    }))
}

/// A group, or a chat with someone in the contacts (the app sends their names).
fn known(chat: &db::ChatRow, mine: &[String], names: &HashMap<String, String>) -> bool {
    chat.group || chat.participants.iter().filter(|p| !mine.contains(p)).all(|p| names.contains_key(p))
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

/// "+491..." -> "tel:+491...", "a@b.org" -> "mailto:a@b.org"; prefixed handles pass through.
pub fn normalize_handle(h: &str) -> Option<String> {
    let h = h.trim();
    if h.starts_with("tel:") || h.starts_with("mailto:") {
        return Some(h.to_lowercase());
    }
    if h.contains('@') {
        return Some(format!("mailto:{}", h.to_lowercase()));
    }
    let phone: String = h.chars().filter(|c| !matches!(c, ' ' | '-' | '(' | ')')).collect();
    let digits = phone.strip_prefix('+').unwrap_or(&phone);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        return Some(format!("tel:{phone}"));
    }
    None
}

pub fn display_handle(handle: &str, names: &HashMap<String, String>) -> String {
    names.get(handle).cloned().unwrap_or_else(|| {
        handle.trim_start_matches("tel:").trim_start_matches("mailto:").to_string()
    })
}

fn title(participants: &[String], name: Option<String>, mine: &[String], names: &HashMap<String, String>) -> String {
    name.filter(|n| !n.is_empty()).unwrap_or_else(|| {
        participants.iter()
            .filter(|h| !mine.contains(h))
            .map(|h| display_handle(&h, names))
            .collect::<Vec<_>>()
            .join(", ")
    })
}

// as OpenBubbles
const SEARCH_CHATS: usize = 10;
const SEARCH_MESSAGES: usize = 50;

fn failed(e: impl std::fmt::Display) -> fdo::Error {
    fdo::Error::Failed(e.to_string())
}

fn invalid(msg: &str) -> fdo::Error {
    fdo::Error::InvalidArgs(msg.to_string())
}

impl Api {
    /// Our own message, if it may still be changed within `window` ms.
    fn own_message(&self, id: &str, window: i64) -> fdo::Result<(db::MessageRow, String)> {
        let (m, chat) = self.db.lock().unwrap().message(id).map_err(failed)?.ok_or_else(|| invalid("no such message"))?;
        if !m.sender.is_empty() || m.unsent {
            return Err(invalid("not a message of ours"));
        }
        if now_ms() - m.ts > window {
            return Err(invalid("too late to change this message"));
        }
        Ok((m, chat))
    }

    async fn queue(&self, out: Outgoing) -> fdo::Result<()> {
        self.outbox.send(out).await.map_err(failed)
    }

    /// The chat to send into: `to` is either one existing chat id or handles,
    /// which find or make the chat with them and us. Also checks the message
    /// replied to is in it.
    fn outgoing_chat(&self, to: &[String], reply_to: String) -> fdo::Result<(String, Option<String>)> {
        let chat_id = {
            let db = self.db.lock().unwrap();
            match to {
                [id] if db.chat_exists(id).map_err(failed)? => id.clone(),
                _ => {
                    let mut participants = to.iter()
                        .map(|h| normalize_handle(h).ok_or_else(|| invalid(&format!("not a phone number or email: {h}"))))
                        .collect::<fdo::Result<Vec<_>>>()?;
                    if participants.is_empty() {
                        return Err(invalid("no recipients"));
                    }
                    participants.push(self.me.clone());
                    match db.find_chat(None, &participants).map_err(failed)? {
                        Some(id) => id,
                        None => db.create_chat(&participants, None, false).map_err(failed)?,
                    }
                }
            }
        };
        self.member(&chat_id)?;
        let reply_to = (!reply_to.is_empty()).then_some(reply_to);
        if let Some(r) = &reply_to {
            let same_chat = self.db.lock().unwrap().message(r).map_err(failed)?.is_some_and(|(_, c)| c == chat_id);
            if !same_chat {
                return Err(invalid("reply to a message of another chat"));
            }
        }
        Ok((chat_id, reply_to))
    }

    fn ours(&self, handle: &str) -> bool {
        self.mine.iter().any(|m| m == handle)
    }

    /// The chat's participants, us included; fails once we left it.
    fn member(&self, chat_id: &str) -> fdo::Result<Vec<String>> {
        let participants = self.db.lock().unwrap().chat_participants(chat_id).map_err(failed)?;
        if !participants.iter().any(|p| self.ours(p)) {
            return Err(invalid("you are no longer in this chat"));
        }
        Ok(participants)
    }

    /// A group chat we are still in, with its participants.
    fn group(&self, chat_id: &str) -> fdo::Result<Vec<String>> {
        let participants = self.member(chat_id)?;
        if !self.db.lock().unwrap().group_info(chat_id).map_err(failed)?.0 {
            return Err(invalid("not a group chat"));
        }
        Ok(participants)
    }
}

#[interface(name = "org.sailbubbles.Daemon1")]
impl Api {
    /// (id, participants other than us, title, last text, last time in ms,
    /// unread, pinned, muted, group photo path or "", whether we left, group
    /// name or "", archived, known (a group, or someone in the contacts),
    /// group), pinned first, then newest first.
    #[allow(clippy::type_complexity)]
    fn chats(&self) -> fdo::Result<Vec<(String, Vec<String>, String, String, i64, u32, bool, bool, String, bool, String,
                                        bool, bool, bool)>> {
        let db = self.db.lock().unwrap();
        let rows = db.chats().map_err(failed)?;
        let names = db.names().map_err(failed)?;
        Ok(rows.into_iter().map(|c| {
            let name = c.name.clone().unwrap_or_default();
            let known = known(&c, &self.mine, &names);
            let left = !c.participants.iter().any(|p| self.ours(p));
            let title = title(&c.participants, c.name, &self.mine, &names);
            let others = c.participants.into_iter().filter(|p| !self.ours(p)).collect();
            (c.id, others, title, c.last_text, c.last_ts, c.unread, c.pinned, c.muted, c.icon.unwrap_or_default(), left,
             name, c.archived, known, c.group)
        }).collect())
    }

    /// An archived chat is listed apart; only this device knows.
    async fn set_archived(&self, chat_id: String, archived: bool,
                          #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> fdo::Result<()> {
        if !self.db.lock().unwrap().set_archived(&chat_id, archived).map_err(failed)? {
            return Err(invalid("no such chat"));
        }
        Api::chats_changed(&emitter).await.map_err(failed)
    }

    /// Earlier texts of an edited message as (text, since when in ms), oldest first.
    fn edit_history(&self, message_id: String) -> fdo::Result<Vec<(String, i64)>> {
        self.db.lock().unwrap().edit_history(&message_id).map_err(failed)
    }

    /// The chat's participants other than us, as (handle, display name).
    fn participants(&self, chat_id: String) -> fdo::Result<Vec<(String, String)>> {
        let db = self.db.lock().unwrap();
        let names = db.names().map_err(failed)?;
        Ok(db.chat_participants(&chat_id).map_err(failed)?.into_iter()
            .filter(|p| !self.ours(p))
            .map(|p| { let name = display_handle(&p, &names); (p, name) })
            .collect())
    }

    /// Photos and videos of the chat on the phone, newest first, as
    /// (attachment id, name, mime, path).
    fn chat_media(&self, chat_id: String) -> fdo::Result<Vec<(String, String, String, String)>> {
        Ok(self.db.lock().unwrap().media(&chat_id).map_err(failed)?.into_iter()
            .map(|(message_id, a)| (attachments::id(&message_id, a.idx), a.name, a.mime, a.path.unwrap_or_default()))
            .collect())
    }

    /// Renames a group, or removes its name with "".
    async fn rename_group(&self, chat_id: String, name: String) -> fdo::Result<()> {
        self.group(&chat_id)?;
        self.queue(Outgoing::Rename { chat_id, name: name.trim().to_string() }).await
    }

    /// Adds a phone number or email to a group; ActionFailed tells when it
    /// cannot get iMessages.
    async fn add_participant(&self, chat_id: String, handle: String) -> fdo::Result<()> {
        let mut participants = self.group(&chat_id)?;
        let handle = normalize_handle(&handle).ok_or_else(|| invalid("not a phone number or email"))?;
        if participants.contains(&handle) {
            return Err(invalid("already in this group"));
        }
        participants.push(handle.clone());
        self.queue(Outgoing::Participants { chat_id, participants, change: (db::Event::Added, handle) }).await
    }

    async fn remove_participant(&self, chat_id: String, handle: String) -> fdo::Result<()> {
        let mut participants = self.group(&chat_id)?;
        if self.ours(&handle) || !participants.contains(&handle) {
            return Err(invalid("not someone else in this group"));
        }
        participants.retain(|p| *p != handle);
        self.queue(Outgoing::Participants { chat_id, participants, change: (db::Event::Removed, handle) }).await
    }

    async fn leave_group(&self, chat_id: String) -> fdo::Result<()> {
        let mut participants = self.group(&chat_id)?;
        let me = participants.iter().find(|p| self.ours(p)).cloned().unwrap_or_else(|| self.me.clone());
        participants.retain(|p| !self.ours(p));
        self.queue(Outgoing::Participants { chat_id, participants, change: (db::Event::Left, me) }).await
    }

    /// Sets the group photo from a 570x570 PNG the app made, or removes it with "".
    async fn set_group_photo(&self, chat_id: String, path: String) -> fdo::Result<()> {
        self.group(&chat_id)?;
        let png = if path.is_empty() { None } else {
            Some(attachments::group_photo_source(&self.home, &path).map_err(|e| invalid(&e.to_string()))?)
        };
        self.queue(Outgoing::Icon { chat_id, png }).await
    }

    /// Chats whose title contains `term` (message id ""), then the newest
    /// messages containing it, as (chat id, title, message id, text, time in ms).
    #[allow(clippy::type_complexity)]
    fn search(&self, term: String) -> fdo::Result<Vec<(String, String, String, String, i64)>> {
        let term = term.trim();
        if term.is_empty() {
            return Ok(vec![]);
        }
        let lower = term.to_lowercase();
        let db = self.db.lock().unwrap();
        let names = db.names().map_err(failed)?;
        let mut titles = HashMap::new();
        let mut found = Vec::new();
        for c in db.chats().map_err(failed)? {
            let title = title(&c.participants, c.name, &self.mine, &names);
            if found.len() < SEARCH_CHATS && title.to_lowercase().contains(&lower) {
                found.push((c.id.clone(), title.clone(), String::new(), c.last_text, c.last_ts));
            }
            titles.insert(c.id, title);
        }
        for (chat, m) in db.search(term, SEARCH_MESSAGES).map_err(failed)? {
            let title = titles.get(&chat).cloned().unwrap_or_default();
            found.push((chat, title, m.id, m.text, m.ts));
        }
        Ok(found)
    }

    /// The chat's unsent text and picked file paths, kept on this device only.
    fn draft(&self, chat_id: String) -> fdo::Result<(String, Vec<String>)> {
        self.db.lock().unwrap().draft(&chat_id).map_err(failed)
    }

    /// Replaces the chat's draft; empty text and no files remove it.
    fn set_draft(&self, chat_id: String, text: String, paths: Vec<String>) -> fdo::Result<()> {
        self.db.lock().unwrap().set_draft(&chat_id, &text, &paths).map_err(failed)
    }

    /// Contact names for chat partners, keyed by handle; replaces earlier ones.
    fn set_names(&self, names: HashMap<String, String>) -> fdo::Result<()> {
        self.db.lock().unwrap().set_names(&names).map_err(failed)
    }

    /// Starts over in sign-in mode, where the app signs in to Apple again.
    async fn sign_in_again(&self) -> fdo::Result<()> {
        store::write_private(&self.state_dir.join(crate::SIGNIN_MARKER), b"").map_err(failed)?;
        // after the reply has gone out
        tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let e = crate::restart();
            log::error!("restarting failed: {e}");
            std::process::exit(1);
        });
        Ok(())
    }

    /// Clears the chat's unread count and its notification. Sends no read receipt.
    async fn mark_read(&self, chat_id: String, #[zbus(connection)] bus: &zbus::Connection) -> fdo::Result<()> {
        if !self.db.lock().unwrap().mark_read(&chat_id).map_err(failed)? {
            return Err(fdo::Error::InvalidArgs("no such chat".into()));
        }
        if let Err(e) = self.notifier.clear(bus, &chat_id).await {
            log::error!("closing a notification failed: {e}");
        }
        self.outbox.send(Outgoing::Read { chat_id }).await.map_err(failed)
    }

    /// (push: connected/connecting/offline, registration: ok/renewing/retrying/
    /// signin/stopped, next registration renewal in ms or 0).
    fn status(&self) -> (String, String, i64) {
        let h = self.health.lock().unwrap();
        (h.push.to_string(), h.registration.to_string(), h.renew_at_ms)
    }

    fn settings(&self) -> HashMap<String, bool> {
        let db = self.db.lock().unwrap();
        SETTINGS.iter().map(|(k, _)| (k.to_string(), setting(&db, k))).collect()
    }

    fn set_setting(&self, key: String, value: bool) -> fdo::Result<()> {
        if !SETTINGS.iter().any(|(k, _)| *k == key) {
            return Err(fdo::Error::InvalidArgs(format!("unknown setting: {key}")));
        }
        self.db.lock().unwrap().set_setting(&key, value).map_err(failed)
    }

    /// (id, sender ("" = me), sender name, text, time in ms, status, reactions as
    /// (who, kind), reply-to id, reply-to text, edited, unsent, attachments as
    /// (id, name, mime, size, local path or ""), kind), newest first, older than
    /// `before` (0 = newest). Reactions from us have an empty who. A kind other
    /// than "" is a group event (renamed, added, removed, left, icon,
    /// iconRemoved) whose text is the new name or the name of the person
    /// concerned ("" = us).
    #[allow(clippy::type_complexity)]
    fn messages(&self, chat_id: String, before: i64, limit: u32)
        -> fdo::Result<Vec<(String, String, String, String, i64, u32, Vec<(String, String)>, String, String, bool, bool,
                            Vec<(String, String, String, i64, String)>, String)>>
    {
        let db = self.db.lock().unwrap();
        let names = db.names().map_err(failed)?;
        let who = |h: &str| if h.is_empty() { String::new() } else { display_handle(h, &names) };
        db.messages(&chat_id, before, limit).map_err(failed)?.into_iter().map(|m| {
            let reactions = db.reactions(&m.id).map_err(failed)?.into_iter()
                .map(|(sender, kind)| (who(&sender), kind)).collect();
            let reply_to = m.reply_to.unwrap_or_default();
            let reply_text = if reply_to.is_empty() { String::new() } else {
                db.message(&reply_to).map_err(failed)?.map(|(r, _)| r.text).unwrap_or_default()
            };
            let files = db.attachments(&m.id).map_err(failed)?.into_iter()
                .map(|a| (attachments::id(&m.id, a.idx), a.name, a.mime, a.size, a.path.unwrap_or_default()))
                .collect();
            let text = match m.kind.as_str() {
                "added" | "removed" | "left" => if self.ours(&m.text) { String::new() } else { who(&m.text) },
                _ => m.text,
            };
            Ok((m.id, m.sender.clone(), who(&m.sender), text, m.ts, m.status, reactions,
                reply_to, reply_text, m.edited, m.unsent, files, m.kind))
        }).collect()
    }

    /// Sets our reaction (one of REACTIONS or "emoji:<e>"), or removes it with "".
    async fn react(&self, message_id: String, kind: String,
                   #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> fdo::Result<()> {
        if !kind.is_empty() && !REACTIONS.contains(&kind.as_str()) && !kind.starts_with("emoji:") {
            return Err(invalid("unknown reaction"));
        }
        let (enable, previous) = {
            let db = self.db.lock().unwrap();
            let previous = db.reactions(&message_id).map_err(failed)?.into_iter()
                .find(|(sender, _)| sender.is_empty()).map(|(_, k)| k);
            (!kind.is_empty(), previous)
        };
        let Some(chat_id) = self.db.lock().unwrap()
            .set_reaction(&message_id, "", enable.then_some(kind.as_str())).map_err(failed)?
        else {
            return Err(invalid("no such message"));
        };
        // Apple removes a tapback by sending the same kind with enable off.
        let sent_kind = if enable { kind } else { match previous { Some(k) => k, None => return Ok(()) } };
        Api::message_received(&emitter, &chat_id, &message_id).await.map_err(failed)?;
        self.queue(Outgoing::React { chat_id, message_id, kind: sent_kind, enable }).await
    }

    /// Edits a message of ours, at most 15 minutes after sending.
    async fn edit(&self, message_id: String, text: String,
                  #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> fdo::Result<()> {
        if text.trim().is_empty() {
            return Err(invalid("empty message"));
        }
        let (_, chat_id) = self.own_message(&message_id, EDIT_WINDOW_MS)?;
        self.db.lock().unwrap().edit_message(&message_id, &text, now_ms()).map_err(failed)?;
        Api::message_received(&emitter, &chat_id, &message_id).await.map_err(failed)?;
        self.queue(Outgoing::Edit { chat_id, message_id, text }).await
    }

    /// Takes a message of ours back from everyone, at most 2 minutes after sending.
    async fn unsend(&self, message_id: String,
                    #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> fdo::Result<()> {
        let (_, chat_id) = self.own_message(&message_id, UNSEND_WINDOW_MS)?;
        self.db.lock().unwrap().unsend_message(&message_id).map_err(failed)?;
        Api::message_received(&emitter, &chat_id, &message_id).await.map_err(failed)?;
        self.queue(Outgoing::Unsend { chat_id, message_id }).await
    }

    async fn typing(&self, chat_id: String, typing: bool) -> fdo::Result<()> {
        self.queue(Outgoing::Typing { chat_id, typing }).await
    }

    /// Fetches an attachment that was too large to fetch on arrival, or failed.
    async fn download(&self, attachment_id: String) -> fdo::Result<()> {
        let (message_id, idx) = attachments::parse_id(&attachment_id).ok_or_else(|| invalid("not an attachment id"))?;
        if self.db.lock().unwrap().attachment_info(message_id, idx).map_err(failed)?.is_none() {
            return Err(invalid("no such attachment"));
        }
        self.downloads.send((message_id.to_string(), idx)).await.map_err(failed)
    }

    /// Deletes here and on our other devices; the other side keeps it.
    async fn delete_message(&self, message_id: String,
                            #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> fdo::Result<()> {
        let Some(chat_id) = self.db.lock().unwrap().delete_message(&message_id).map_err(failed)? else {
            return Err(invalid("no such message"));
        };
        attachments::remove(&self.attachments_root, &message_id);
        Api::message_received(&emitter, &chat_id, &message_id).await.map_err(failed)?;
        self.queue(Outgoing::DeleteMessage { message_id }).await
    }

    /// Deletes the chat here and on our other devices; the other side keeps it.
    async fn delete_chat(&self, chat_id: String, #[zbus(connection)] bus: &zbus::Connection,
                         #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> fdo::Result<()> {
        let (guid, participants) = {
            let mut db = self.db.lock().unwrap();
            let (_, guid) = db.chat_meta(&chat_id).map_err(failed)?;
            let participants = db.chat_participants(&chat_id).map_err(failed)?;
            for id in db.message_ids(&chat_id).map_err(failed)? {
                attachments::remove(&self.attachments_root, &id);
            }
            if !db.delete_chat(&chat_id).map_err(failed)? {
                return Err(invalid("no such chat"));
            }
            (guid, participants)
        };
        if let Err(e) = self.notifier.clear(bus, &chat_id).await {
            log::error!("closing a notification failed: {e}");
        }
        Api::chats_changed(&emitter).await.map_err(failed)?;
        self.queue(Outgoing::DeleteChat { participants, guid }).await
    }

    async fn set_pinned(&self, chat_id: String, pinned: bool,
                        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> fdo::Result<()> {
        if !self.db.lock().unwrap().set_pinned(&chat_id, pinned).map_err(failed)? {
            return Err(invalid("no such chat"));
        }
        Api::chats_changed(&emitter).await.map_err(failed)
    }

    /// A muted chat posts no notifications; only this device knows.
    async fn set_muted(&self, chat_id: String, muted: bool,
                       #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> fdo::Result<()> {
        if !self.db.lock().unwrap().set_muted(&chat_id, muted).map_err(failed)? {
            return Err(invalid("no such chat"));
        }
        Api::chats_changed(&emitter).await.map_err(failed)
    }

    /// Marks the chat unread here and on our other devices.
    async fn mark_unread(&self, chat_id: String,
                         #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> fdo::Result<()> {
        let latest = {
            let db = self.db.lock().unwrap();
            if !db.mark_unread(&chat_id).map_err(failed)? {
                return Err(invalid("no such chat"));
            }
            db.messages(&chat_id, 0, 1).map_err(failed)?.into_iter().next()
        };
        Api::chats_changed(&emitter).await.map_err(failed)?;
        match latest {
            Some(m) => self.queue(Outgoing::MarkUnread { message_id: m.id }).await,
            None => Ok(()),
        }
    }

    /// Sends `text` to `to` (phone numbers, emails or tel:/mailto: handles),
    /// as a reply to `reply_to` unless it is empty; returns (chat id, message id).
    async fn send(&self, to: Vec<String>, text: String, reply_to: String) -> fdo::Result<(String, String)> {
        if text.trim().is_empty() {
            return Err(fdo::Error::InvalidArgs("empty message".into()));
        }
        let (chat_id, reply_to) = self.outgoing_chat(&to, reply_to)?;
        let id = new_uuid().map_err(failed)?;
        self.db.lock().unwrap().add_message(&db::Message {
            id: &id, chat_id: &chat_id, sender: "", text: &text, ts: now_ms(), status: Status::Sending,
            reply_to: reply_to.as_deref(),
        }, None, None).map_err(failed)?;
        self.queue(Outgoing::Text { id: id.clone(), chat_id: chat_id.clone(), text, reply_to }).await?;
        Ok((chat_id, id))
    }

    /// Sends the file at `path` (of MIME type `mime`, "" if unknown) as a
    /// message of its own, as OpenBubbles does; returns (chat id, message id).
    async fn send_file(&self, to: Vec<String>, path: String, mime: String, reply_to: String)
        -> fdo::Result<(String, String)>
    {
        let file = attachments::sendable(&self.home, &path).map_err(|e| invalid(&e.to_string()))?;
        let (chat_id, reply_to) = self.outgoing_chat(&to, reply_to)?;
        let id = new_uuid().map_err(failed)?;
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mime = if mime.is_empty() { "application/octet-stream".to_string() } else { mime };
        // A file of our own keeps the chat whole if the original is deleted.
        let copy = attachments::path_for(&self.attachments_root, &id, 0, &name);
        store::ensure_private_dir(copy.parent().unwrap_or(&self.attachments_root)).map_err(failed)?;
        attachments::keep(&file, &copy).await.map_err(failed)?;
        let size = std::fs::metadata(&copy).map_err(failed)?.len();
        {
            let db = self.db.lock().unwrap();
            db.add_message(&db::Message {
                id: &id, chat_id: &chat_id, sender: "", text: "\u{fffc}", ts: now_ms(), status: Status::Sending,
                reply_to: reply_to.as_deref(),
            }, None, None).map_err(failed)?;
            db.add_attachment(&id, 0, &name, &mime, size as i64, &[]).map_err(failed)?;
            db.set_attachment_file(&id, 0, &copy.to_string_lossy(), &mime).map_err(failed)?;
        }
        self.queue(Outgoing::File { id: id.clone(), chat_id: chat_id.clone(), path: copy, name, mime, reply_to }).await?;
        Ok((chat_id, id))
    }

    #[zbus(signal)]
    pub async fn message_received(e: &SignalEmitter<'_>, chat_id: &str, message_id: &str) -> zbus::Result<()>;

    /// The chat was read on one of our other devices.
    #[zbus(signal)]
    pub async fn chat_read(e: &SignalEmitter<'_>, chat_id: &str) -> zbus::Result<()>;

    /// Chats were deleted, pinned or marked unread.
    #[zbus(signal)]
    pub async fn chats_changed(e: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn status_changed(e: &SignalEmitter<'_>) -> zbus::Result<()>;

    /// Someone in the chat started or stopped typing.
    #[zbus(signal)]
    pub async fn typing_changed(e: &SignalEmitter<'_>, chat_id: &str, typing: bool) -> zbus::Result<()>;

    /// A group change (rename, add, remove, leave, photo) could not be sent.
    #[zbus(signal)]
    pub async fn action_failed(e: &SignalEmitter<'_>, chat_id: &str, action: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn message_status(e: &SignalEmitter<'_>, chat_id: &str, message_id: &str, status: u32) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixStream;
    use zbus::connection::Builder;

    const ME: &str = "mailto:me@example.org";
    const ALIAS: &str = "mailto:alias@example.org";
    type ChatTuple = (String, Vec<String>, String, String, i64, u32, bool, bool, String, bool, String, bool, bool, bool);
    type MsgTuple = (String, String, String, String, i64, u32, Vec<(String, String)>, String, String, bool, bool,
                     Vec<(String, String, String, i64, String)>, String);

    async fn connect(db: Db) -> (zbus::Connection, mpsc::Receiver<Outgoing>) {
        connect_with_downloads(db).await.0
    }

    #[allow(clippy::type_complexity)]
    async fn connect_with_downloads(db: Db)
        -> ((zbus::Connection, mpsc::Receiver<Outgoing>), mpsc::Receiver<(String, u32)>)
    {
        let (tx, rx) = mpsc::channel(8);
        let (dl_tx, dl_rx) = mpsc::channel(8);
        let home = home();
        let api = Api { db: Arc::new(Mutex::new(db)), me: ME.into(), mine: vec![ME.into(), ALIAS.into()], outbox: tx,
                       notifier: Default::default(),
                       health: Default::default(), downloads: dl_tx, attachments_root: home.join("attachments"),
                       state_dir: home.clone(), home };
        let (a, b) = UnixStream::pair().unwrap();
        let server = Builder::unix_stream(a).server(zbus::Guid::generate()).unwrap().p2p()
            .serve_at(PATH, api).unwrap().build();
        let client = Builder::unix_stream(b).p2p().build();
        let (server, client) = tokio::try_join!(server, client).unwrap();
        std::mem::forget(server);
        ((client, rx), dl_rx)
    }

    fn home() -> PathBuf {
        std::env::temp_dir().join(format!("obd-api-{}", std::process::id()))
    }

    async fn call<B, R>(c: &zbus::Connection, method: &str, body: &B) -> zbus::Result<R>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
    {
        let reply = c.call_method(None::<&str>, PATH, Some("org.sailbubbles.Daemon1"), method, body).await?;
        reply.body().deserialize()
    }

    #[test]
    fn handles_are_normalized() {
        assert_eq!(normalize_handle("+49 151 2345-678").as_deref(), Some("tel:+491512345678"));
        assert_eq!(normalize_handle("Friend@Example.org").as_deref(), Some("mailto:friend@example.org"));
        assert_eq!(normalize_handle("tel:+1555").as_deref(), Some("tel:+1555"));
        assert_eq!(normalize_handle("hello"), None);
        assert_eq!(normalize_handle("+"), None);
    }

    #[test]
    fn title_leaves_out_me() {
        let p: Vec<String> = vec![ME.into(), "mailto:a@b.org".into(), "tel:+1555".into()];
        let (none, mine) = (HashMap::new(), [ME.to_string()]);
        assert_eq!(title(&p, None, &mine, &none), "a@b.org, +1555");
        assert_eq!(title(&p, Some("Group".into()), &mine, &none), "Group");
        assert_eq!(title(&p, Some(String::new()), &mine, &none), "a@b.org, +1555");
        let names = HashMap::from([("tel:+1555".to_string(), "Anna".to_string())]);
        assert_eq!(title(&p, None, &mine, &names), "a@b.org, Anna");
    }

    #[tokio::test]
    async fn send_stores_and_queues() {
        let (c, mut rx) = connect(Db::in_memory()).await;
        let (chat, id): (String, String) = call(&c, "Send", &(vec!["+1555"], "hi", "")).await.unwrap();
        assert_eq!(chat, db::chat_id(&[ME.into(), "tel:+1555".into()]));

        let Outgoing::Text { id: out_id, chat_id: out_chat, text, reply_to: None } = rx.recv().await.unwrap() else { panic!("not a text") };
        assert_eq!((out_id, out_chat, text.as_str()), (id.clone(), chat.clone(), "hi"));

        let chats: Vec<ChatTuple> = call(&c, "Chats", &()).await.unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!((chats[0].2.as_str(), chats[0].3.as_str()), ("+1555", "hi"));
        let msgs: Vec<MsgTuple> = call(&c, "Messages", &(chat.clone(), 0i64, 10u32)).await.unwrap();
        assert_eq!((msgs[0].1.as_str(), msgs[0].2.as_str(), msgs[0].5), ("", "", Status::Sending as u32));
        call::<_, ()>(&c, "MarkRead", &(chat.clone(),)).await.unwrap();
        assert!(matches!(rx.recv().await.unwrap(), Outgoing::Read { chat_id } if chat_id == chat));

        let names = HashMap::from([("tel:+1555", "Anna")]);
        call::<_, ()>(&c, "SetNames", &(names,)).await.unwrap();
        let chats: Vec<ChatTuple> = call(&c, "Chats", &()).await.unwrap();
        assert_eq!(chats[0].2, "Anna");
        assert!(call::<_, ()>(&c, "MarkRead", &("nope",)).await.is_err());
    }

    #[tokio::test]
    async fn groups_change_through_the_queue() {
        let db = Db::in_memory();
        let group = db.create_chat(&[ME.into(), "tel:+1".into(), "tel:+2".into()], Some("Climbing"), false).unwrap();
        let one = db.create_chat(&[ME.into(), "tel:+1".into()], None, false).unwrap();
        db.add_event("E", &group, "tel:+1", db::Event::Added, ME, 1).unwrap();
        let (c, mut rx) = connect(db).await;

        let chats: Vec<ChatTuple> = call(&c, "Chats", &()).await.unwrap();
        let g = chats.iter().find(|t| t.0 == group).unwrap();
        assert_eq!((g.1.clone(), g.2.as_str(), g.9, g.10.as_str()),
                   (vec!["tel:+1".to_string(), "tel:+2".to_string()], "Climbing", false, "Climbing"));
        let people: Vec<(String, String)> = call(&c, "Participants", &(group.clone(),)).await.unwrap();
        assert_eq!(people, [("tel:+1".to_string(), "+1".to_string()), ("tel:+2".to_string(), "+2".to_string())]);
        let msgs: Vec<MsgTuple> = call(&c, "Messages", &(group.clone(), 0i64, 10u32)).await.unwrap();
        assert_eq!((msgs[0].12.as_str(), msgs[0].3.as_str(), msgs[0].2.as_str()), ("added", "", "+1"));

        call::<_, ()>(&c, "RenameGroup", &(group.clone(), " Bouldering ")).await.unwrap();
        assert!(matches!(rx.recv().await.unwrap(), Outgoing::Rename { chat_id, name } if chat_id == group && name == "Bouldering"));
        call::<_, ()>(&c, "AddParticipant", &(group.clone(), "+3")).await.unwrap();
        let Outgoing::Participants { participants, change, .. } = rx.recv().await.unwrap() else { panic!("not a change") };
        assert_eq!((participants.len(), change), (4, (db::Event::Added, "tel:+3".to_string())));
        assert!(call::<_, ()>(&c, "AddParticipant", &(group.clone(), "+2")).await.is_err());
        call::<_, ()>(&c, "RemoveParticipant", &(group.clone(), "tel:+2")).await.unwrap();
        let Outgoing::Participants { participants, change, .. } = rx.recv().await.unwrap() else { panic!("not a change") };
        assert_eq!((participants, change.0), (vec![ME.to_string(), "tel:+1".to_string()], db::Event::Removed));
        assert!(call::<_, ()>(&c, "RemoveParticipant", &(group.clone(), ME)).await.is_err());
        call::<_, ()>(&c, "LeaveGroup", &(group.clone(),)).await.unwrap();
        let Outgoing::Participants { participants, change, .. } = rx.recv().await.unwrap() else { panic!("not a change") };
        assert_eq!((participants.contains(&ME.to_string()), change.0), (false, db::Event::Left));
        call::<_, ()>(&c, "SetGroupPhoto", &(group.clone(), "")).await.unwrap();
        assert!(matches!(rx.recv().await.unwrap(), Outgoing::Icon { png: None, .. }));
        assert!(call::<_, ()>(&c, "SetGroupPhoto", &(group.clone(), "/etc/passwd")).await.is_err());
        assert!(call::<_, ()>(&c, "RenameGroup", &(one.clone(), "x")).await.is_err());

        let (sent_to, _): (String, String) = call(&c, "Send", &(vec![group.clone()], "hi", "")).await.unwrap();
        assert_eq!(sent_to, group);
        assert!(matches!(rx.recv().await.unwrap(), Outgoing::Text { chat_id, .. } if chat_id == group));
    }

    #[tokio::test]
    async fn chats_with_our_other_handle_and_unknown_people() {
        let db = Db::in_memory();
        let alias_chat = db.create_chat(&[ALIAS.into(), "tel:+1".into()], None, false).unwrap();
        db.add_message(&db::Message { id: "m", chat_id: &alias_chat, sender: "tel:+1", text: "hi", ts: 1,
                                      status: Status::Received, reply_to: None }, None, None).unwrap();
        db.edit_message("m", "hi there", 2).unwrap();
        let (c, mut rx) = connect(db).await;
        let chats: Vec<ChatTuple> = call(&c, "Chats", &()).await.unwrap();
        assert_eq!((chats[0].1.clone(), chats[0].9, chats[0].11, chats[0].12, chats[0].13),
                   (vec!["tel:+1".to_string()], false, false, false, false));
        call::<_, ()>(&c, "SetNames", &(HashMap::from([("tel:+1", "Anna")]),)).await.unwrap();
        call::<_, ()>(&c, "SetArchived", &(alias_chat.clone(), true)).await.unwrap();
        let chats: Vec<ChatTuple> = call(&c, "Chats", &()).await.unwrap();
        assert_eq!((chats[0].11, chats[0].12), (true, true));
        let history: Vec<(String, i64)> = call(&c, "EditHistory", &("m",)).await.unwrap();
        assert_eq!(history, [("hi".to_string(), 1)]);
        let (sent_to, _): (String, String) = call(&c, "Send", &(vec![alias_chat.clone()], "yes", "")).await.unwrap();
        assert_eq!(sent_to, alias_chat);
        assert!(matches!(rx.recv().await.unwrap(), Outgoing::Text { .. }));
    }

    #[tokio::test]
    async fn a_group_we_left_takes_no_messages() {
        let db = Db::in_memory();
        let group = db.create_chat(&["tel:+1".into(), "tel:+2".into(), "tel:+3".into()], None, false).unwrap();
        let (c, _rx) = connect(db).await;
        let chats: Vec<ChatTuple> = call(&c, "Chats", &()).await.unwrap();
        assert!(chats[0].9);
        assert!(call::<_, (String, String)>(&c, "Send", &(vec![group.clone()], "hi", "")).await.is_err());
        assert!(call::<_, ()>(&c, "RenameGroup", &(group, "x")).await.is_err());
    }

    #[tokio::test]
    async fn settings_round_trip() {
        let (c, _rx) = connect(Db::in_memory()).await;
        let s: HashMap<String, bool> = call(&c, "Settings", &()).await.unwrap();
        assert_eq!(s, HashMap::from([("previews".to_string(), true), ("readReceipts".to_string(), false),
                                     ("filterUnknown".to_string(), false), ("unarchiveOnNewMessage".to_string(), false)]));
        call::<_, ()>(&c, "SetSetting", &("previews", false)).await.unwrap();
        let s: HashMap<String, bool> = call(&c, "Settings", &()).await.unwrap();
        assert!(!s["previews"]);
        assert!(call::<_, ()>(&c, "SetSetting", &("nope", true)).await.is_err());
    }

    #[tokio::test]
    async fn messages_carry_sender_names_and_reactions() {
        let db = Db::in_memory();
        let chat = db::chat_id(&[ME.into(), "tel:+1".into(), "tel:+2".into()]);
        db.add_message(&db::Message { id: "A", chat_id: &chat, sender: "tel:+1", text: "hi", ts: 1, status: Status::Received, reply_to: None },
                       Some("G"), None).unwrap();
        db.set_reaction("A", "tel:+2", Some("heart")).unwrap();
        db.set_reaction("A", "", Some("like")).unwrap();
        let mut db = db;
        db.set_names(&HashMap::from([("tel:+1".into(), "Anna".into()), ("tel:+2".into(), "Ben".into())])).unwrap();
        let (c, _rx) = connect(db).await;
        let msgs: Vec<MsgTuple> = call(&c, "Messages", &(chat, 0i64, 10u32)).await.unwrap();
        assert_eq!(msgs[0].2, "Anna");
        assert_eq!(msgs[0].6, [("Ben".to_string(), "heart".to_string()), (String::new(), "like".to_string())]);
    }

    fn own(db: &Db, chat: &str, id: &str, age_ms: i64) {
        db.add_message(&db::Message { id, chat_id: chat, sender: "", text: id, ts: now_ms() - age_ms,
                                      status: Status::Sent, reply_to: None }, None, None).unwrap();
    }

    #[tokio::test]
    async fn edit_and_unsend_keep_apples_time_limits() {
        let db = Db::in_memory();
        let chat = db::chat_id(&[ME.into(), "tel:+1".into()]);
        own(&db, &chat, "fresh", 1000);
        own(&db, &chat, "old", 20 * 60 * 1000);
        own(&db, &chat, "minutes", 5 * 60 * 1000);
        db.add_message(&db::Message { id: "theirs", chat_id: &chat, sender: "tel:+1", text: "x", ts: now_ms(),
                                      status: Status::Received, reply_to: None }, None, None).unwrap();
        let (c, mut rx) = connect(db).await;

        call::<_, ()>(&c, "Edit", &("fresh", "better")).await.unwrap();
        assert!(matches!(rx.recv().await.unwrap(), Outgoing::Edit { message_id, text, .. } if message_id == "fresh" && text == "better"));
        assert!(call::<_, ()>(&c, "Edit", &("old", "late")).await.is_err());
        assert!(call::<_, ()>(&c, "Edit", &("theirs", "mine now")).await.is_err());

        assert!(call::<_, ()>(&c, "Unsend", &("minutes",)).await.is_err(), "past the 2 minute limit");
        call::<_, ()>(&c, "Unsend", &("fresh",)).await.unwrap();
        assert!(matches!(rx.recv().await.unwrap(), Outgoing::Unsend { message_id, .. } if message_id == "fresh"));
        assert!(call::<_, ()>(&c, "Edit", &("fresh", "again")).await.is_err(), "unsent stays unsent");

        let msgs: Vec<MsgTuple> = call(&c, "Messages", &(chat, 0i64, 10u32)).await.unwrap();
        let fresh = msgs.iter().find(|m| m.0 == "fresh").unwrap();
        assert_eq!((fresh.3.as_str(), fresh.9, fresh.10), ("", true, true));
    }

    #[tokio::test]
    async fn reactions_replies_and_deletes() {
        let db = Db::in_memory();
        let chat = db::chat_id(&[ME.into(), "tel:+1555".into()]);
        own(&db, &chat, "A", 1000);
        let (c, mut rx) = connect(db).await;

        call::<_, ()>(&c, "React", &("A", "heart")).await.unwrap();
        assert!(matches!(rx.recv().await.unwrap(), Outgoing::React { kind, enable: true, .. } if kind == "heart"));
        call::<_, ()>(&c, "React", &("A", "")).await.unwrap();
        assert!(matches!(rx.recv().await.unwrap(), Outgoing::React { kind, enable: false, .. } if kind == "heart"));
        assert!(call::<_, ()>(&c, "React", &("A", "bogus")).await.is_err());

        let (_, id): (String, String) = call(&c, "Send", &(vec!["+1555"], "re", "A")).await.unwrap();
        assert!(matches!(rx.recv().await.unwrap(), Outgoing::Text { reply_to: Some(r), .. } if r == "A"));
        let msgs: Vec<MsgTuple> = call(&c, "Messages", &(chat.clone(), 0i64, 10u32)).await.unwrap();
        let reply = msgs.iter().find(|m| m.0 == id).unwrap();
        assert_eq!((reply.7.as_str(), reply.8.as_str()), ("A", "A"));

        call::<_, ()>(&c, "DeleteMessage", &("A",)).await.unwrap();
        assert!(matches!(rx.recv().await.unwrap(), Outgoing::DeleteMessage { message_id } if message_id == "A"));
        call::<_, ()>(&c, "SetPinned", &(chat.clone(), true)).await.unwrap();
        let chats: Vec<ChatTuple> = call(&c, "Chats", &()).await.unwrap();
        assert!(chats[0].6);
        let found: Vec<(String, String, String, String, i64)> = call(&c, "Search", &("RE",)).await.unwrap();
        assert!(found.iter().any(|f| f.0 == chat && f.2 == id && f.3 == "re"));
        let found: Vec<(String, String, String, String, i64)> = call(&c, "Search", &("+1555",)).await.unwrap();
        assert_eq!((found[0].0.as_str(), found[0].2.as_str()), (chat.as_str(), ""));
        let found: Vec<(String, String, String, String, i64)> = call(&c, "Search", &("  ",)).await.unwrap();
        assert!(found.is_empty());
        call::<_, ()>(&c, "SetDraft", &(chat.clone(), "later", vec!["/p/a.jpg"])).await.unwrap();
        let draft: (String, Vec<String>) = call(&c, "Draft", &(chat.clone(),)).await.unwrap();
        assert_eq!(draft, ("later".to_string(), vec!["/p/a.jpg".to_string()]));
        call::<_, ()>(&c, "SetMuted", &(chat.clone(), true)).await.unwrap();
        let chats: Vec<ChatTuple> = call(&c, "Chats", &()).await.unwrap();
        assert!(chats[0].7);
        call::<_, ()>(&c, "MarkUnread", &(chat.clone(),)).await.unwrap();
        assert!(matches!(rx.recv().await.unwrap(), Outgoing::MarkUnread { message_id, .. } if message_id == id));
        call::<_, ()>(&c, "DeleteChat", &(chat.clone(),)).await.unwrap();
        assert!(matches!(rx.recv().await.unwrap(), Outgoing::DeleteChat { participants, .. } if participants.len() == 2));
        let chats: Vec<ChatTuple> = call(&c, "Chats", &()).await.unwrap();
        assert!(chats.is_empty());
    }

    #[tokio::test]
    async fn status_reports_health() {
        let (c, _rx) = connect(Db::in_memory()).await;
        let s: (String, String, i64) = call(&c, "Status", &()).await.unwrap();
        assert_eq!(s, (String::new(), String::new(), 0));
    }

    #[tokio::test]
    async fn attachments_are_listed_and_fetched_on_request() {
        let db = Db::in_memory();
        let chat = db::chat_id(&[ME.into(), "tel:+1".into()]);
        db.add_message(&db::Message { id: "A", chat_id: &chat, sender: "tel:+1", text: "\u{fffc}", ts: 1,
                                      status: Status::Received, reply_to: None }, None, None).unwrap();
        db.add_attachment("A", 0, "big.mov", "video/quicktime", 99_000_000, b"i").unwrap();
        let ((c, _rx), mut downloads) = connect_with_downloads(db).await;
        let msgs: Vec<MsgTuple> = call(&c, "Messages", &(chat, 0i64, 10u32)).await.unwrap();
        assert_eq!(msgs[0].11, [("A:0".to_string(), "big.mov".to_string(), "video/quicktime".to_string(), 99_000_000, String::new())]);
        call::<_, ()>(&c, "Download", &("A:0",)).await.unwrap();
        assert_eq!(downloads.recv().await.unwrap(), ("A".to_string(), 0));
        assert!(call::<_, ()>(&c, "Download", &("A:7",)).await.is_err());
        assert!(call::<_, ()>(&c, "Download", &("junk",)).await.is_err());
    }

    #[tokio::test]
    async fn files_are_copied_stored_and_queued() {
        let (c, mut rx) = connect(Db::in_memory()).await;
        let pictures = home().join("Pictures");
        std::fs::create_dir_all(&pictures).unwrap();
        let photo = pictures.join("photo one.jpg");
        std::fs::write(&photo, b"jpeg").unwrap();

        let (chat, id): (String, String) = call(&c, "SendFile", &(vec!["+1555"], photo.to_str().unwrap(), "image/jpeg", "")).await.unwrap();
        let Outgoing::File { id: out_id, path, name, mime, .. } = rx.recv().await.unwrap() else { panic!("not a file") };
        assert_eq!((out_id, name.as_str(), mime.as_str()), (id.clone(), "photo one.jpg", "image/jpeg"));
        assert_eq!(path, home().join("attachments").join(&id).join("0-photo_one.jpg"));
        assert_eq!(std::fs::read(&path).unwrap(), b"jpeg");

        let msgs: Vec<MsgTuple> = call(&c, "Messages", &(chat, 0i64, 10u32)).await.unwrap();
        assert_eq!(msgs[0].5, Status::Sending as u32);
        assert_eq!(msgs[0].11, [(format!("{id}:0"), "photo one.jpg".to_string(), "image/jpeg".to_string(), 4,
                                 path.to_string_lossy().into_owned())]);

        let secret = home().join("secret");
        std::fs::write(&secret, b"x").unwrap();
        assert!(call::<_, (String, String)>(&c, "SendFile", &(vec!["+1555"], secret.to_str().unwrap(), "", "")).await.is_err());
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn bad_sends_are_refused() {
        let (c, mut rx) = connect(Db::in_memory()).await;
        assert!(call::<_, (String, String)>(&c, "Send", &(vec!["+1555"], "  ", "")).await.is_err());
        assert!(call::<_, (String, String)>(&c, "Send", &(Vec::<String>::new(), "hi", "")).await.is_err());
        assert!(call::<_, (String, String)>(&c, "Send", &(vec!["nobody"], "hi", "")).await.is_err());
        assert!(rx.try_recv().is_err());
    }
}
