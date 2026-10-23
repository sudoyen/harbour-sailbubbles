use std::collections::HashMap;
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

// Ordered: a status only ever moves forward, so a late delivered receipt
// cannot undo read. Received is for messages from others and never changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Received = 0,
    Sending = 1,
    Sent = 2,
    Delivered = 3,
    Read = 4,
    Failed = 5,
}

pub struct Message<'a> {
    pub id: &'a str,
    pub chat_id: &'a str,
    /// Empty when the message is ours.
    pub sender: &'a str,
    pub text: &'a str,
    /// Milliseconds since the Unix epoch.
    pub ts: i64,
    pub status: Status,
    /// The message this one replies to.
    pub reply_to: Option<&'a str>,
}

#[derive(Debug, PartialEq)]
pub struct ChatRow {
    pub id: String,
    pub participants: Vec<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub group: bool,
    pub archived: bool,
    pub last_text: String,
    pub last_ts: i64,
    pub unread: u32,
    pub pinned: bool,
    pub muted: bool,
}

#[derive(Debug, PartialEq)]
pub struct AttachmentRow {
    pub idx: u32,
    pub name: String,
    pub mime: String,
    pub size: i64,
    pub path: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct MessageRow {
    pub id: String,
    pub sender: String,
    pub text: String,
    pub ts: i64,
    pub status: u32,
    pub reply_to: Option<String>,
    pub edited: bool,
    pub unsent: bool,
    /// "" for a message, else a group event (see Event).
    pub kind: String,
}

/// Group changes, stored as messages of their own kind; the text holds the
/// new name or the handle concerned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Renamed,
    Added,
    Removed,
    Left,
    Icon,
    IconRemoved,
}

impl Event {
    pub fn kind(self) -> &'static str {
        match self {
            Event::Renamed => "renamed",
            Event::Added => "added",
            Event::Removed => "removed",
            Event::Left => "left",
            Event::Icon => "icon",
            Event::IconRemoved => "iconRemoved",
        }
    }
}

const MESSAGE_COLUMNS: &str = "id, sender, text, ts, status, reply_to, edited, unsent, kind";

fn message_row(r: &rusqlite::Row) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: r.get(0)?,
        sender: r.get(1)?,
        text: r.get(2)?,
        ts: r.get(3)?,
        status: r.get(4)?,
        reply_to: r.get(5)?,
        edited: r.get(6)?,
        unsent: r.get(7)?,
        kind: r.get(8)?,
    })
}

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS chats (
        id TEXT PRIMARY KEY,
        name TEXT,
        -- the conversation's sender GUID; group chats need it on every send
        guid TEXT,
        last_ts INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS messages (
        id TEXT PRIMARY KEY,
        chat_id TEXT NOT NULL REFERENCES chats(id),
        sender TEXT NOT NULL,
        text TEXT NOT NULL,
        ts INTEGER NOT NULL,
        status INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS messages_by_chat ON messages(chat_id, ts);
";

// Index = schema version it upgrades from (PRAGMA user_version).
const MIGRATIONS: &[&str] = &[
    "ALTER TABLE chats ADD COLUMN read_ts INTEGER NOT NULL DEFAULT 0;",
    // contact names for chat partners only, pushed by the app (the daemon
    // cannot read the contacts database)
    "CREATE TABLE names (handle TEXT PRIMARY KEY, name TEXT NOT NULL);",
    "CREATE TABLE settings (key TEXT PRIMARY KEY, value INTEGER NOT NULL);",
    // one tapback per person and message, as on Apple devices
    "CREATE TABLE reactions (message_id TEXT NOT NULL, sender TEXT NOT NULL, kind TEXT NOT NULL,
                             PRIMARY KEY (message_id, sender));",
    "ALTER TABLE messages ADD COLUMN reply_to TEXT;
     ALTER TABLE messages ADD COLUMN edited INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE messages ADD COLUMN unsent INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE chats ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE chats ADD COLUMN marked_unread INTEGER NOT NULL DEFAULT 0;",
    // info: the plist-encoded rustpush Attachment, needed to download it later
    "CREATE TABLE attachments (message_id TEXT NOT NULL, idx INTEGER NOT NULL, name TEXT NOT NULL,
                               mime TEXT NOT NULL, size INTEGER NOT NULL, path TEXT, info BLOB NOT NULL,
                               PRIMARY KEY (message_id, idx));",
    // local only, as in OpenBubbles: Apple devices keep their own
    "ALTER TABLE chats ADD COLUMN muted INTEGER NOT NULL DEFAULT 0;",
    // by chat id, not tied to a chats row: a chat can be new or deleted
    "CREATE TABLE drafts (chat_id TEXT PRIMARY KEY, text TEXT NOT NULL);
     CREATE TABLE draft_files (chat_id TEXT NOT NULL, idx INTEGER NOT NULL, path TEXT NOT NULL,
                               PRIMARY KEY (chat_id, idx));",
    // Group members change, so a chat keeps its first id as a plain key and
    // its participants apart. Apple names a group by sender GUIDs, of which a
    // chat can collect several.
    "ALTER TABLE chats ADD COLUMN participants TEXT;
     UPDATE chats SET participants = id;
     ALTER TABLE chats ADD COLUMN group_version INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE chats ADD COLUMN icon TEXT;
     CREATE TABLE chat_guids (guid TEXT PRIMARY KEY, chat_id TEXT NOT NULL);
     INSERT OR IGNORE INTO chat_guids SELECT lower(guid), id FROM chats WHERE guid IS NOT NULL;
     ALTER TABLE messages ADD COLUMN kind TEXT NOT NULL DEFAULT '';",
    // A group stays one when down to two members, so it is never taken for the
    // one-to-one chat with the same people. members_ts: when the members last
    // changed, so an older message cannot change them back. edits: earlier
    // texts of edited messages, with when each was current.
    "ALTER TABLE chats ADD COLUMN is_group INTEGER NOT NULL DEFAULT 0;
     UPDATE chats SET is_group = 1
         WHERE length(participants) - length(replace(participants, ',', '')) >= 2 OR name IS NOT NULL;
     ALTER TABLE chats ADD COLUMN members_ts INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE chats ADD COLUMN archived INTEGER NOT NULL DEFAULT 0;
     CREATE TABLE edits (message_id TEXT NOT NULL, text TEXT NOT NULL, since INTEGER NOT NULL);
     ALTER TABLE messages ADD COLUMN edited_ts INTEGER;",
];

// Each step and its version number commit together, so a daemon stopped
// halfway does not find a half-done step next time.
fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        conn.execute_batch(&format!("BEGIN; {sql} PRAGMA user_version = {}; COMMIT;", i + 1))?;
    }
    Ok(())
}

fn is_group_list(participants: &[String]) -> bool {
    chat_id(participants).split(',').count() > 2
}

/// Sorted, lowercased participant handles, which never contain a comma; a new
/// chat takes this as its id.
pub fn chat_id(participants: &[String]) -> String {
    let mut p: Vec<String> = participants.iter().map(|h| h.to_lowercase()).collect();
    p.sort();
    p.dedup();
    p.join(",")
}

fn split_participants(joined: &str) -> Vec<String> {
    joined.split(',').map(str::to_string).collect()
}

pub struct Db(Connection);

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Db> {
        // create it 0600 before SQLite does it with the default umask
        let _ = OpenOptions::new().write(true).create(true).mode(0o600).open(path);
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        // a send still queued when the daemon stopped never went out
        conn.execute("UPDATE messages SET status = ?1 WHERE status = ?2",
                     params![Status::Failed as u32, Status::Sending as u32])?;
        Ok(Db(conn))
    }

    #[cfg(test)]
    pub fn in_memory() -> Db {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        migrate(&conn).unwrap();
        Db(conn)
    }

    /// Returns false if the message was already stored.
    /// Returns false if the message was already stored. A chat not stored yet
    /// is created with the participants its id lists.
    pub fn add_message(&self, m: &Message, chat_name: Option<&str>, chat_guid: Option<&str>) -> rusqlite::Result<bool> {
        self.0.execute(
            "INSERT INTO chats (id, participants, name, guid, last_ts, is_group)
             VALUES (?1, ?1, ?2, ?3, ?4, length(?1) - length(replace(?1, ',', '')) >= 2)
             ON CONFLICT(id) DO UPDATE SET
                 name = coalesce(excluded.name, name),
                 guid = coalesce(guid, excluded.guid),
                 last_ts = max(last_ts, excluded.last_ts)",
            params![m.chat_id, chat_name, chat_guid, m.ts],
        )?;
        if let Some(guid) = chat_guid {
            self.add_guid(m.chat_id, guid)?;
        }
        let added = self.0.execute(
            "INSERT OR IGNORE INTO messages (id, chat_id, sender, text, ts, status, reply_to)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![m.id, m.chat_id, m.sender, m.text, m.ts, m.status as u32, m.reply_to],
        )?;
        Ok(added == 1)
    }

    /// Stores a group event as a message of its own kind; false if already stored.
    pub fn add_event(&self, id: &str, chat_id: &str, sender: &str, event: Event, text: &str, ts: i64)
        -> rusqlite::Result<bool>
    {
        let status = if sender.is_empty() { Status::Sent } else { Status::Received };
        let added = self.0.execute(
            "INSERT OR IGNORE INTO messages (id, chat_id, sender, text, ts, status, kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, chat_id, sender, text, ts, status as u32, event.kind()],
        )?;
        self.0.execute("UPDATE chats SET last_ts = max(last_ts, ?2) WHERE id = ?1", params![chat_id, ts])?;
        Ok(added == 1)
    }

    /// The chat Apple means by this sender GUID or, without a match, the one
    /// with exactly these participants that is a group when they are more than two.
    pub fn find_chat(&self, guid: Option<&str>, participants: &[String]) -> rusqlite::Result<Option<String>> {
        self.find(guid, participants, is_group_list(participants))
    }

    /// As find_chat, for what only groups get: a group of two is still one.
    pub fn find_group(&self, guid: Option<&str>, participants: &[String]) -> rusqlite::Result<Option<String>> {
        self.find(guid, participants, true)
    }

    fn find(&self, guid: Option<&str>, participants: &[String], group: bool) -> rusqlite::Result<Option<String>> {
        if let Some(guid) = guid {
            let by_guid = self.0.query_row("SELECT chat_id FROM chat_guids WHERE guid = lower(?1)", [guid],
                                           |r| r.get(0)).optional()?;
            if by_guid.is_some() {
                return Ok(by_guid);
            }
        }
        self.0.query_row("SELECT id FROM chats WHERE participants = ?1 AND is_group = ?2 ORDER BY last_ts DESC LIMIT 1",
                         params![chat_id(participants), group], |r| r.get(0)).optional()
    }

    /// Creates a chat with these participants, a group if `group` or they are
    /// more than two; its id is their list, unless an older chat whose members
    /// changed already holds that id.
    pub fn create_chat(&self, participants: &[String], name: Option<&str>, group: bool) -> rusqlite::Result<String> {
        let joined = chat_id(participants);
        let mut id = joined.clone();
        let mut n = 1;
        while self.chat_exists(&id)? {
            n += 1;
            id = format!("{joined}#{n}");
        }
        self.0.execute("INSERT INTO chats (id, participants, name, last_ts, is_group) VALUES (?1, ?2, ?3, 0, ?4)",
                       params![id, joined, name, group || is_group_list(participants)])?;
        Ok(id)
    }

    pub fn add_guid(&self, chat_id: &str, guid: &str) -> rusqlite::Result<()> {
        self.0.execute("INSERT OR IGNORE INTO chat_guids (guid, chat_id) VALUES (lower(?1), ?2)", [guid, chat_id])?;
        self.0.execute("UPDATE chats SET guid = ?2 WHERE id = ?1 AND guid IS NULL", [chat_id, guid])?;
        Ok(())
    }

    pub fn chat_participants(&self, chat_id: &str) -> rusqlite::Result<Vec<String>> {
        let joined: Option<String> = self.0.query_row("SELECT participants FROM chats WHERE id = ?1", [chat_id],
                                                      |r| r.get(0)).optional()?;
        Ok(split_participants(joined.as_deref().unwrap_or(chat_id)))
    }

    /// Records the members as of `ts` (ms); a group stays a group.
    pub fn set_participants(&self, chat: &str, participants: &[String], ts: i64) -> rusqlite::Result<()> {
        self.0.execute(
            "UPDATE chats SET participants = ?2, is_group = max(is_group, ?3), members_ts = max(members_ts, ?4)
             WHERE id = ?1",
            params![chat, chat_id(participants), is_group_list(participants), ts])?;
        Ok(())
    }

    /// Whether the chat is a group, and when its members last changed.
    pub fn group_info(&self, chat: &str) -> rusqlite::Result<(bool, i64)> {
        Ok(self.0.query_row("SELECT is_group, members_ts FROM chats WHERE id = ?1", [chat],
                            |r| Ok((r.get(0)?, r.get(1)?))).optional()?.unwrap_or((false, 0)))
    }

    pub fn set_archived(&self, chat_id: &str, archived: bool) -> rusqlite::Result<bool> {
        Ok(self.0.execute("UPDATE chats SET archived = ?2 WHERE id = ?1", params![chat_id, archived])? == 1)
    }

    /// Sets `archived` only where it was `was`; returns whether it changed.
    pub fn set_archived_if(&self, chat_id: &str, was: bool, archived: bool) -> rusqlite::Result<bool> {
        Ok(self.0.execute("UPDATE chats SET archived = ?3 WHERE id = ?1 AND archived = ?2",
                          params![chat_id, was, archived])? == 1)
    }

    /// Earlier texts of the message as (text, since when in ms), oldest first.
    pub fn edit_history(&self, message_id: &str) -> rusqlite::Result<Vec<(String, i64)>> {
        let mut q = self.0.prepare("SELECT text, since FROM edits WHERE message_id = ?1 ORDER BY rowid")?;
        let rows = q.query_map([message_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    pub fn set_name(&self, chat_id: &str, name: Option<&str>) -> rusqlite::Result<()> {
        self.0.execute("UPDATE chats SET name = ?2 WHERE id = ?1", params![chat_id, name])?;
        Ok(())
    }

    pub fn group_version(&self, chat_id: &str) -> rusqlite::Result<u64> {
        Ok(self.0.query_row("SELECT group_version FROM chats WHERE id = ?1", [chat_id], |r| r.get::<_, i64>(0))
            .optional()?.unwrap_or(0) as u64)
    }

    pub fn set_group_version(&self, chat_id: &str, version: u64) -> rusqlite::Result<()> {
        self.0.execute("UPDATE chats SET group_version = max(group_version, ?2) WHERE id = ?1",
                       params![chat_id, version as i64])?;
        Ok(())
    }

    pub fn set_icon(&self, chat_id: &str, path: Option<&str>) -> rusqlite::Result<()> {
        self.0.execute("UPDATE chats SET icon = ?2 WHERE id = ?1", params![chat_id, path])?;
        Ok(())
    }

    /// Points stored attachment and group photo paths under `old` to `new`,
    /// after the folder was moved.
    pub fn move_files(&self, old: &str, new: &str) -> rusqlite::Result<()> {
        self.0.execute("UPDATE attachments SET path = ?2 || substr(path, length(?1) + 1)
                        WHERE substr(path, 1, length(?1)) = ?1", [old, new])?;
        self.0.execute("UPDATE chats SET icon = ?2 || substr(icon, length(?1) + 1)
                        WHERE substr(icon, 1, length(?1)) = ?1", [old, new])?;
        Ok(())
    }

    /// Photos and videos of the chat that are on the phone, newest first, as
    /// (message id, attachment row).
    pub fn media(&self, chat_id: &str) -> rusqlite::Result<Vec<(String, AttachmentRow)>> {
        let mut q = self.0.prepare(
            "SELECT a.message_id, a.idx, a.name, a.mime, a.size, a.path FROM attachments a
             JOIN messages m ON m.id = a.message_id
             WHERE m.chat_id = ?1 AND a.path IS NOT NULL AND (a.mime LIKE 'image/%' OR a.mime LIKE 'video/%')
             ORDER BY m.ts DESC, a.idx")?;
        let rows = q.query_map([chat_id], |r| Ok((r.get(0)?, AttachmentRow {
            idx: r.get(1)?, name: r.get(2)?, mime: r.get(3)?, size: r.get(4)?, path: r.get(5)?,
        })))?;
        rows.collect()
    }

    /// Moves a message of ours forward to `status`; returns false if nothing changed.
    pub fn set_status(&self, id: &str, status: Status) -> rusqlite::Result<bool> {
        let changed = self.0.execute(
            "UPDATE messages SET status = ?2
             WHERE id = ?1 AND status != ?3 AND status < ?2",
            params![id, status as u32, Status::Received as u32],
        )?;
        Ok(changed == 1)
    }

    pub fn chat_exists(&self, chat_id: &str) -> rusqlite::Result<bool> {
        Ok(self.0.query_row("SELECT 1 FROM chats WHERE id = ?1", [chat_id], |_| Ok(())).optional()?.is_some())
    }

    /// The chat's (name, sender GUID), as needed to send into it.
    pub fn chat_meta(&self, chat_id: &str) -> rusqlite::Result<(Option<String>, Option<String>)> {
        self.0.query_row("SELECT name, guid FROM chats WHERE id = ?1", [chat_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional().map(Option::unwrap_or_default)
    }

    pub fn chat_of(&self, message_id: &str) -> rusqlite::Result<Option<String>> {
        self.0.query_row("SELECT chat_id FROM messages WHERE id = ?1", [message_id], |r| r.get(0))
            .optional()
    }

    pub fn chats(&self) -> rusqlite::Result<Vec<ChatRow>> {
        let mut q = self.0.prepare(
            "SELECT c.id, c.name, c.last_ts,
                    coalesce((SELECT text FROM messages m WHERE m.chat_id = c.id AND m.kind = ''
                              ORDER BY ts DESC LIMIT 1), ''),
                    (SELECT count(*) FROM messages m WHERE m.chat_id = c.id
                     AND m.sender != '' AND m.kind = '' AND m.ts > c.read_ts),
                    c.marked_unread, c.pinned, c.muted, coalesce(c.participants, c.id), c.icon, c.is_group, c.archived
             FROM chats c ORDER BY c.pinned DESC, c.last_ts DESC",
        )?;
        let rows = q.query_map([], |r| {
            let (count, marked): (u32, bool) = (r.get(4)?, r.get(5)?);
            Ok(ChatRow {
                id: r.get(0)?,
                name: r.get(1)?,
                last_ts: r.get(2)?,
                last_text: r.get(3)?,
                // a chat marked unread counts at least one, as on Apple devices
                unread: if marked { count.max(1) } else { count },
                pinned: r.get(6)?,
                muted: r.get(7)?,
                participants: split_participants(&r.get::<_, String>(8)?),
                icon: r.get(9)?,
                group: r.get(10)?,
                archived: r.get(11)?,
            })
        })?;
        rows.collect()
    }

    /// Marks everything in the chat up to now as read; returns false for an unknown chat.
    pub fn mark_read(&self, chat_id: &str) -> rusqlite::Result<bool> {
        let changed = self.0.execute(
            "UPDATE chats SET marked_unread = 0, read_ts = max(read_ts,
                 coalesce((SELECT max(ts) FROM messages WHERE chat_id = ?1), 0))
             WHERE id = ?1",
            [chat_id],
        )?;
        Ok(changed == 1)
    }

    pub fn mark_unread(&self, chat_id: &str) -> rusqlite::Result<bool> {
        Ok(self.0.execute("UPDATE chats SET marked_unread = 1 WHERE id = ?1", [chat_id])? == 1)
    }

    pub fn set_pinned(&self, chat_id: &str, pinned: bool) -> rusqlite::Result<bool> {
        Ok(self.0.execute("UPDATE chats SET pinned = ?2 WHERE id = ?1", params![chat_id, pinned])? == 1)
    }

    pub fn set_muted(&self, chat_id: &str, muted: bool) -> rusqlite::Result<bool> {
        Ok(self.0.execute("UPDATE chats SET muted = ?2 WHERE id = ?1", params![chat_id, muted])? == 1)
    }

    pub fn muted(&self, chat_id: &str) -> rusqlite::Result<bool> {
        Ok(self.0.query_row("SELECT muted FROM chats WHERE id = ?1", [chat_id], |r| r.get(0))
            .optional()?.unwrap_or(false))
    }

    /// The message and its chat.
    pub fn message(&self, id: &str) -> rusqlite::Result<Option<(MessageRow, String)>> {
        self.0.query_row(&format!("SELECT {MESSAGE_COLUMNS}, chat_id FROM messages WHERE id = ?1"), [id],
                         |r| Ok((message_row(r)?, r.get(9)?)))
            .optional()
    }

    /// Replaces the text at `ts` (ms), keeps the old one, marks the message
    /// edited; returns its chat.
    pub fn edit_message(&self, id: &str, text: &str, ts: i64) -> rusqlite::Result<Option<String>> {
        // the old text was current from the last edit, or from sending
        self.0.execute(
            "INSERT INTO edits (message_id, text, since)
             SELECT id, text, coalesce(edited_ts, ts) FROM messages WHERE id = ?1 AND unsent = 0 AND text != ?2",
            params![id, text])?;
        self.0.execute("UPDATE messages SET text = ?2, edited = 1, edited_ts = ?3 WHERE id = ?1 AND unsent = 0 AND text != ?2",
                       params![id, text, ts])?;
        self.chat_of(id)
    }

    /// Drops the text and marks the message unsent; returns its chat.
    pub fn unsend_message(&self, id: &str) -> rusqlite::Result<Option<String>> {
        self.0.execute("UPDATE messages SET text = '', unsent = 1 WHERE id = ?1", [id])?;
        self.0.execute("DELETE FROM reactions WHERE message_id = ?1", [id])?;
        self.0.execute("DELETE FROM edits WHERE message_id = ?1", [id])?;
        self.chat_of(id)
    }

    pub fn add_attachment(&self, message_id: &str, idx: u32, name: &str, mime: &str, size: i64, info: &[u8])
        -> rusqlite::Result<()>
    {
        self.0.execute(
            "INSERT OR IGNORE INTO attachments (message_id, idx, name, mime, size, info)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![message_id, idx, name, mime, size, info],
        )?;
        Ok(())
    }

    pub fn attachments(&self, message_id: &str) -> rusqlite::Result<Vec<AttachmentRow>> {
        let mut q = self.0.prepare(
            "SELECT idx, name, mime, size, path FROM attachments WHERE message_id = ?1 ORDER BY idx")?;
        let rows = q.query_map([message_id], |r| Ok(AttachmentRow {
            idx: r.get(0)?, name: r.get(1)?, mime: r.get(2)?, size: r.get(3)?, path: r.get(4)?,
        }))?;
        rows.collect()
    }

    /// (download info, file name, type, chat) of an attachment.
    pub fn attachment_info(&self, message_id: &str, idx: u32)
        -> rusqlite::Result<Option<(Vec<u8>, String, String, String)>>
    {
        self.0.query_row(
            "SELECT a.info, a.name, a.mime, m.chat_id FROM attachments a JOIN messages m ON m.id = a.message_id
             WHERE a.message_id = ?1 AND a.idx = ?2",
            params![message_id, idx], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .optional()
    }

    /// Records the downloaded file and the type it ended up as (HEIC becomes JPEG).
    pub fn set_attachment_file(&self, message_id: &str, idx: u32, path: &str, mime: &str) -> rusqlite::Result<()> {
        self.0.execute("UPDATE attachments SET path = ?3, mime = ?4 WHERE message_id = ?1 AND idx = ?2",
                       params![message_id, idx, path, mime])?;
        Ok(())
    }

    /// Message ids of the chat, for removing their attachment files.
    pub fn message_ids(&self, chat_id: &str) -> rusqlite::Result<Vec<String>> {
        let mut q = self.0.prepare("SELECT id FROM messages WHERE chat_id = ?1")?;
        let rows = q.query_map([chat_id], |r| r.get(0))?;
        rows.collect()
    }

    /// Removes the message, its reactions and attachments; returns its chat.
    pub fn delete_message(&self, id: &str) -> rusqlite::Result<Option<String>> {
        let chat = self.chat_of(id)?;
        self.0.execute("DELETE FROM attachments WHERE message_id = ?1", [id])?;
        self.0.execute("DELETE FROM reactions WHERE message_id = ?1", [id])?;
        self.0.execute("DELETE FROM edits WHERE message_id = ?1", [id])?;
        self.0.execute("DELETE FROM messages WHERE id = ?1", [id])?;
        Ok(chat)
    }

    pub fn delete_chat(&mut self, chat_id: &str) -> rusqlite::Result<bool> {
        let tx = self.0.transaction()?;
        tx.execute("DELETE FROM reactions WHERE message_id IN (SELECT id FROM messages WHERE chat_id = ?1)", [chat_id])?;
        tx.execute("DELETE FROM attachments WHERE message_id IN (SELECT id FROM messages WHERE chat_id = ?1)", [chat_id])?;
        tx.execute("DELETE FROM edits WHERE message_id IN (SELECT id FROM messages WHERE chat_id = ?1)", [chat_id])?;
        tx.execute("DELETE FROM messages WHERE chat_id = ?1", [chat_id])?;
        let removed = tx.execute("DELETE FROM chats WHERE id = ?1", [chat_id])?;
        // a later message in this conversation starts a new chat
        tx.execute("DELETE FROM chat_guids WHERE chat_id = ?1", [chat_id])?;
        tx.execute("DELETE FROM drafts WHERE chat_id = ?1", [chat_id])?;
        tx.execute("DELETE FROM draft_files WHERE chat_id = ?1", [chat_id])?;
        tx.commit()?;
        Ok(removed == 1)
    }

    /// Replaces the chat's draft; an empty one is removed.
    pub fn set_draft(&mut self, chat_id: &str, text: &str, paths: &[String]) -> rusqlite::Result<()> {
        let tx = self.0.transaction()?;
        tx.execute("DELETE FROM drafts WHERE chat_id = ?1", [chat_id])?;
        tx.execute("DELETE FROM draft_files WHERE chat_id = ?1", [chat_id])?;
        if !text.is_empty() || !paths.is_empty() {
            tx.execute("INSERT INTO drafts (chat_id, text) VALUES (?1, ?2)", [chat_id, text])?;
            for (i, path) in paths.iter().enumerate() {
                tx.execute("INSERT INTO draft_files (chat_id, idx, path) VALUES (?1, ?2, ?3)",
                           params![chat_id, i as i64, path])?;
            }
        }
        tx.commit()
    }

    pub fn draft(&self, chat_id: &str) -> rusqlite::Result<(String, Vec<String>)> {
        let text = self.0.query_row("SELECT text FROM drafts WHERE chat_id = ?1", [chat_id], |r| r.get(0))
            .optional()?.unwrap_or_default();
        let mut q = self.0.prepare("SELECT path FROM draft_files WHERE chat_id = ?1 ORDER BY idx")?;
        let paths = q.query_map([chat_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok((text, paths))
    }

    /// Up to `limit` messages containing `term`, ignoring case, newest first, as
    /// (chat id, message). SQLite's LIKE ignores case only for ASCII.
    pub fn search(&self, term: &str, limit: usize) -> rusqlite::Result<Vec<(String, MessageRow)>> {
        let term = term.to_lowercase();
        let mut q = self.0.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS}, chat_id FROM messages WHERE unsent = 0 AND kind = '' ORDER BY ts DESC"))?;
        let mut rows = q.query([])?;
        let mut found = Vec::new();
        while found.len() < limit {
            let Some(r) = rows.next()? else { break };
            let text: String = r.get(2)?;
            if text.to_lowercase().contains(&term) {
                found.push((r.get(9)?, message_row(r)?));
            }
        }
        Ok(found)
    }

    pub fn setting(&self, key: &str, default: bool) -> rusqlite::Result<bool> {
        self.0.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional().map(|v| v.unwrap_or(default))
    }

    pub fn set_setting(&self, key: &str, value: bool) -> rusqlite::Result<()> {
        self.0.execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)", params![key, value])?;
        Ok(())
    }

    /// Sets or, with `kind` None, removes `sender`'s reaction; returns the
    /// reacted-to message's chat when it is stored here.
    pub fn set_reaction(&self, message_id: &str, sender: &str, kind: Option<&str>) -> rusqlite::Result<Option<String>> {
        match kind {
            Some(kind) => self.0.execute(
                "INSERT OR REPLACE INTO reactions (message_id, sender, kind) VALUES (?1, ?2, ?3)",
                params![message_id, sender, kind])?,
            None => self.0.execute(
                "DELETE FROM reactions WHERE message_id = ?1 AND sender = ?2", params![message_id, sender])?,
        };
        self.chat_of(message_id)
    }

    /// (sender, kind) pairs, in the order they came.
    pub fn reactions(&self, message_id: &str) -> rusqlite::Result<Vec<(String, String)>> {
        let mut q = self.0.prepare("SELECT sender, kind FROM reactions WHERE message_id = ?1 ORDER BY rowid")?;
        let rows = q.query_map([message_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// Replaces all stored contact names.
    pub fn set_names(&mut self, names: &HashMap<String, String>) -> rusqlite::Result<()> {
        let tx = self.0.transaction()?;
        tx.execute("DELETE FROM names", [])?;
        for (handle, name) in names {
            tx.execute("INSERT OR REPLACE INTO names (handle, name) VALUES (?1, ?2)",
                       params![handle.to_lowercase(), name])?;
        }
        tx.commit()
    }

    pub fn names(&self) -> rusqlite::Result<HashMap<String, String>> {
        let mut q = self.0.prepare("SELECT handle, name FROM names")?;
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// Newest first, older than `before` (0 = from the newest).
    pub fn messages(&self, chat_id: &str, before: i64, limit: u32) -> rusqlite::Result<Vec<MessageRow>> {
        let before = if before == 0 { i64::MAX } else { before };
        let mut q = self.0.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages
             WHERE chat_id = ?1 AND ts < ?2 ORDER BY ts DESC LIMIT ?3"))?;
        let rows = q.query_map(params![chat_id, before, limit], message_row)?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg<'a>(id: &'a str, chat: &'a str, ts: i64, status: Status) -> Message<'a> {
        Message { id, chat_id: chat, sender: "", text: id, ts, status, reply_to: None }
    }

    #[test]
    fn chat_id_ignores_order_and_case() {
        let a = chat_id(&["tel:+491".into(), "mailto:Me@x.org".into()]);
        let b = chat_id(&["mailto:me@x.org".into(), "tel:+491".into(), "tel:+491".into()]);
        assert_eq!(a, b);
        assert_eq!(split_participants(&a), vec!["mailto:me@x.org", "tel:+491"]);
    }

    fn handles(list: &[&str]) -> Vec<String> {
        list.iter().map(|h| h.to_string()).collect()
    }

    #[test]
    fn chats_from_before_groups_keep_their_participants_and_guid() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let before = MIGRATIONS.iter().position(|m| m.contains("chat_guids")).unwrap();
        for sql in &MIGRATIONS[..before] {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute_batch(&format!("PRAGMA user_version = {before}")).unwrap();
        conn.execute("INSERT INTO chats (id, guid, last_ts) VALUES ('me,tel:+1,tel:+2', 'G-1', 1)", []).unwrap();
        conn.execute("INSERT INTO messages (id, chat_id, sender, text, ts, status) VALUES ('m', 'me,tel:+1,tel:+2', '', 'hi', 1, 2)",
                     []).unwrap();
        migrate(&conn).unwrap();
        let db = Db(conn);
        assert_eq!(db.chat_participants("me,tel:+1,tel:+2").unwrap(), ["me", "tel:+1", "tel:+2"]);
        assert_eq!(db.find_chat(Some("G-1"), &[]).unwrap().as_deref(), Some("me,tel:+1,tel:+2"));
        assert_eq!(db.messages("me,tel:+1,tel:+2", 0, 1).unwrap()[0].kind, "");
    }

    #[test]
    fn chats_are_found_by_guid_then_by_participants() {
        let db = Db::in_memory();
        let (old, new) = (handles(&["mailto:me", "tel:+1", "tel:+2"]), handles(&["mailto:me", "tel:+1", "tel:+3"]));
        let id = chat_id(&old);
        db.add_message(&msg("a", &id, 1, Status::Received), None, Some("G-1")).unwrap();
        assert_eq!(db.find_chat(None, &old).unwrap().as_deref(), Some(id.as_str()));
        assert_eq!(db.find_chat(Some("g-1"), &new).unwrap().as_deref(), Some(id.as_str()));
        assert_eq!(db.find_chat(Some("other"), &new).unwrap(), None);
        db.set_participants(&id, &new, 2).unwrap();
        assert_eq!(db.chat_participants(&id).unwrap(), chat_id(&new).split(',').collect::<Vec<_>>());
        assert_eq!(db.find_chat(None, &new).unwrap().as_deref(), Some(id.as_str()));
        assert_eq!(db.find_chat(None, &old).unwrap(), None);
    }

    #[test]
    fn a_deleted_chat_forgets_its_guids() {
        let mut db = Db::in_memory();
        let group = handles(&["mailto:me", "tel:+1", "tel:+2"]);
        let id = chat_id(&group);
        db.add_message(&msg("a", &id, 1, Status::Received), None, Some("G")).unwrap();
        assert!(db.delete_chat(&id).unwrap());
        assert_eq!(db.find_chat(Some("G"), &handles(&["mailto:me", "tel:+9"])).unwrap(), None);
    }

    #[test]
    fn a_group_down_to_two_is_not_the_one_to_one_chat() {
        let db = Db::in_memory();
        let two = handles(&["mailto:me", "tel:+1"]);
        let one = db.create_chat(&two, None, false).unwrap();
        let group = db.create_chat(&handles(&["mailto:me", "tel:+1", "tel:+2"]), None, false).unwrap();
        db.set_participants(&group, &two, 5).unwrap();
        assert_eq!(db.group_info(&group).unwrap(), (true, 5));
        assert_eq!(db.find_chat(None, &two).unwrap().as_deref(), Some(one.as_str()));
        assert_eq!(db.find_group(None, &two).unwrap().as_deref(), Some(group.as_str()));
        assert_eq!(db.group_info(&one).unwrap().0, false);
    }

    #[test]
    fn moved_files_keep_their_paths() {
        let db = Db::in_memory();
        db.add_message(&msg("a", "c", 1, Status::Received), None, None).unwrap();
        db.add_attachment("a", 0, "p.jpg", "image/jpeg", 1, &[]).unwrap();
        db.set_attachment_file("a", 0, "/old/files/a/0-p.jpg", "image/jpeg").unwrap();
        db.set_icon("c", Some("/old/files/_groups/x-1.png")).unwrap();
        db.move_files("/old/files", "/new/files").unwrap();
        assert_eq!(db.attachments("a").unwrap()[0].path.as_deref(), Some("/new/files/a/0-p.jpg"));
        assert_eq!(db.chats().unwrap()[0].icon.as_deref(), Some("/new/files/_groups/x-1.png"));
    }

    #[test]
    fn archiving_is_per_chat() {
        let db = Db::in_memory();
        db.add_message(&msg("a", "c", 1, Status::Sent), None, None).unwrap();
        assert!(db.set_archived("c", true).unwrap());
        assert!(db.chats().unwrap()[0].archived);
        assert!(!db.set_archived_if("c", false, true).unwrap());
        assert!(db.set_archived_if("c", true, false).unwrap());
        assert!(!db.chats().unwrap()[0].archived);
        assert!(!db.set_archived("nope", true).unwrap());
    }

    #[test]
    fn edits_keep_the_earlier_texts() {
        let db = Db::in_memory();
        db.add_message(&msg("first", "c", 1, Status::Sent), None, None).unwrap();
        db.edit_message("first", "second", 5).unwrap();
        db.edit_message("first", "second", 6).unwrap();
        db.edit_message("first", "third", 9).unwrap();
        assert_eq!(db.edit_history("first").unwrap(), [("first".to_string(), 1), ("second".to_string(), 5)]);
        db.unsend_message("first").unwrap();
        assert!(db.edit_history("first").unwrap().is_empty(), "unsent text is gone, also its history");
    }

    #[test]
    fn a_new_chat_does_not_take_an_id_in_use() {
        let db = Db::in_memory();
        let (old, new) = (handles(&["mailto:me", "tel:+1", "tel:+2"]), handles(&["mailto:me", "tel:+1", "tel:+3"]));
        let first = db.create_chat(&old, Some("Climbing"), false).unwrap();
        assert_eq!(first, chat_id(&old));
        db.set_participants(&first, &new, 2).unwrap();
        let second = db.create_chat(&old, None, false).unwrap();
        assert_eq!(second, format!("{}#2", chat_id(&old)));
        assert_eq!(db.chat_participants(&second).unwrap(), db.chat_participants(&chat_id(&old)).map(|_| {
            chat_id(&old).split(',').map(str::to_string).collect::<Vec<_>>()
        }).unwrap());
        assert_eq!(db.chat_meta(&first).unwrap().0.as_deref(), Some("Climbing"));
    }

    #[test]
    fn group_events_are_neither_unread_nor_the_last_text() {
        let db = Db::in_memory();
        db.add_message(&Message { sender: "tel:+1", ..msg("hi", "c", 1, Status::Received) }, None, None).unwrap();
        db.mark_read("c").unwrap();
        assert!(db.add_event("e", "c", "tel:+1", Event::Renamed, "Climbing", 2).unwrap());
        assert!(!db.add_event("e", "c", "tel:+1", Event::Renamed, "Climbing", 2).unwrap());
        let chat = &db.chats().unwrap()[0];
        assert_eq!((chat.last_text.as_str(), chat.unread, chat.last_ts), ("hi", 0, 2));
        let msgs = db.messages("c", 0, 10).unwrap();
        assert_eq!((msgs[0].kind.as_str(), msgs[0].text.as_str()), ("renamed", "Climbing"));
        assert!(db.search("climbing", 10).unwrap().is_empty());
    }

    #[test]
    fn media_lists_pictures_and_videos_on_the_phone() {
        let db = Db::in_memory();
        db.add_message(&msg("a", "c", 1, Status::Received), None, None).unwrap();
        db.add_message(&msg("b", "c", 2, Status::Received), None, None).unwrap();
        db.add_attachment("a", 0, "p.jpg", "image/jpeg", 1, &[]).unwrap();
        db.add_attachment("a", 1, "t.pdf", "application/pdf", 1, &[]).unwrap();
        db.add_attachment("b", 0, "v.mov", "video/quicktime", 1, &[]).unwrap();
        db.add_attachment("b", 1, "big.jpg", "image/jpeg", 1, &[]).unwrap();
        db.set_attachment_file("a", 0, "/p.jpg", "image/jpeg").unwrap();
        db.set_attachment_file("a", 1, "/t.pdf", "application/pdf").unwrap();
        db.set_attachment_file("b", 0, "/v.mov", "video/quicktime").unwrap();
        let media: Vec<_> = db.media("c").unwrap().into_iter().map(|(m, a)| (m, a.name)).collect();
        assert_eq!(media, [("b".to_string(), "v.mov".to_string()), ("a".to_string(), "p.jpg".to_string())]);
    }

    #[test]
    fn duplicates_are_ignored() {
        let db = Db::in_memory();
        assert!(db.add_message(&msg("A", "c", 1, Status::Received), None, None).unwrap());
        assert!(!db.add_message(&msg("A", "c", 1, Status::Received), None, None).unwrap());
        assert_eq!(db.messages("c", 0, 10).unwrap().len(), 1);
    }

    #[test]
    fn status_only_moves_forward() {
        let db = Db::in_memory();
        db.add_message(&msg("A", "c", 1, Status::Sending), None, None).unwrap();
        assert!(db.set_status("A", Status::Read).unwrap());
        assert!(!db.set_status("A", Status::Delivered).unwrap());
        assert_eq!(db.messages("c", 0, 1).unwrap()[0].status, Status::Read as u32);
    }

    #[test]
    fn received_messages_keep_their_status() {
        let db = Db::in_memory();
        db.add_message(&msg("A", "c", 1, Status::Received), None, None).unwrap();
        assert!(!db.set_status("A", Status::Read).unwrap());
    }

    #[test]
    fn chats_are_newest_first_with_last_text() {
        let db = Db::in_memory();
        db.add_message(&msg("old", "c1", 1, Status::Received), Some("Group"), Some("G1")).unwrap();
        db.add_message(&msg("new", "c2", 5, Status::Sent), None, None).unwrap();
        db.add_message(&msg("older", "c2", 2, Status::Sent), None, None).unwrap();
        let chats = db.chats().unwrap();
        assert_eq!(chats[0], ChatRow { id: "c2".into(), participants: vec!["c2".into()], name: None, icon: None,
                                       group: false, archived: false, last_text: "new".into(), last_ts: 5, unread: 0,
                                       pinned: false, muted: false });
        assert_eq!(chats[1].name.as_deref(), Some("Group"));
        assert_eq!(db.chat_of("older").unwrap().as_deref(), Some("c2"));
        assert_eq!(db.chat_meta("c1").unwrap(), (Some("Group".into()), Some("G1".into())));
        assert_eq!(db.chat_meta("c2").unwrap(), (None, None));
        assert_eq!(db.chat_meta("nope").unwrap(), (None, None));
    }

    #[test]
    fn reopening_fails_unsent_messages() {
        let path = std::env::temp_dir().join(format!("obd-db-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Db::open(&path).unwrap().add_message(&msg("A", "c", 1, Status::Sending), None, None).unwrap();
        let db = Db::open(&path).unwrap();
        assert_eq!(db.messages("c", 0, 1).unwrap()[0].status, Status::Failed as u32);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn unread_counts_messages_from_others_until_marked() {
        let db = Db::in_memory();
        db.add_message(&Message { sender: "tel:+1", ..msg("a", "c", 1, Status::Received) }, None, None).unwrap();
        db.add_message(&Message { sender: "tel:+1", ..msg("b", "c", 2, Status::Received) }, None, None).unwrap();
        db.add_message(&msg("mine", "c", 3, Status::Sent), None, None).unwrap();
        assert_eq!(db.chats().unwrap()[0].unread, 2);
        assert!(db.mark_read("c").unwrap());
        assert_eq!(db.chats().unwrap()[0].unread, 0);
        db.add_message(&Message { sender: "tel:+1", ..msg("c", "c", 4, Status::Received) }, None, None).unwrap();
        assert_eq!(db.chats().unwrap()[0].unread, 1);
        assert!(!db.mark_read("nope").unwrap());
    }

    #[test]
    fn names_are_replaced_as_a_whole() {
        let mut db = Db::in_memory();
        db.set_names(&HashMap::from([("tel:+1".into(), "Anna".into()), ("Mailto:B@x.org".into(), "Ben".into())])).unwrap();
        assert_eq!(db.names().unwrap()["mailto:b@x.org"], "Ben");
        db.set_names(&HashMap::from([("tel:+1".into(), "Anna B".into())])).unwrap();
        assert_eq!(db.names().unwrap(), HashMap::from([("tel:+1".into(), "Anna B".into())]));
    }

    #[test]
    fn settings_fall_back_to_their_default() {
        let db = Db::in_memory();
        assert!(db.setting("previews", true).unwrap());
        db.set_setting("previews", false).unwrap();
        assert!(!db.setting("previews", true).unwrap());
    }

    #[test]
    fn reactions_replace_and_remove() {
        let db = Db::in_memory();
        db.add_message(&msg("A", "c", 1, Status::Sent), None, None).unwrap();
        assert_eq!(db.set_reaction("A", "tel:+1", Some("heart")).unwrap().as_deref(), Some("c"));
        db.set_reaction("A", "tel:+1", Some("like")).unwrap();
        db.set_reaction("A", "tel:+2", Some("emoji:x")).unwrap();
        assert_eq!(db.reactions("A").unwrap(), [("tel:+1".into(), "like".into()), ("tel:+2".into(), "emoji:x".into())]);
        db.set_reaction("A", "tel:+1", None).unwrap();
        assert_eq!(db.reactions("A").unwrap(), [("tel:+2".to_string(), "emoji:x".to_string())]);
        assert_eq!(db.set_reaction("unknown", "tel:+1", Some("heart")).unwrap(), None);
    }

    #[test]
    fn edits_unsends_and_replies_are_kept() {
        let db = Db::in_memory();
        db.add_message(&msg("A", "c", 1, Status::Sent), None, None).unwrap();
        db.add_message(&Message { reply_to: Some("A"), ..msg("B", "c", 2, Status::Received) }, None, None).unwrap();
        db.set_reaction("A", "tel:+1", Some("heart")).unwrap();
        assert_eq!(db.edit_message("A", "fixed", 5).unwrap().as_deref(), Some("c"));
        let (a, chat) = db.message("A").unwrap().unwrap();
        assert_eq!((a.text.as_str(), a.edited, chat.as_str()), ("fixed", true, "c"));
        assert_eq!(db.message("B").unwrap().unwrap().0.reply_to.as_deref(), Some("A"));
        db.unsend_message("A").unwrap();
        let a = db.message("A").unwrap().unwrap().0;
        assert_eq!((a.text.as_str(), a.unsent), ("", true));
        assert!(db.reactions("A").unwrap().is_empty());
        db.edit_message("A", "again", 6).unwrap();
        assert_eq!(db.message("A").unwrap().unwrap().0.text, "", "unsent messages stay unsent");
    }

    #[test]
    fn deleting_messages_and_chats() {
        let mut db = Db::in_memory();
        db.add_message(&msg("A", "c", 1, Status::Sent), None, None).unwrap();
        db.add_message(&msg("B", "c", 2, Status::Sent), None, None).unwrap();
        db.set_reaction("B", "tel:+1", Some("like")).unwrap();
        assert_eq!(db.delete_message("A").unwrap().as_deref(), Some("c"));
        assert_eq!(db.messages("c", 0, 10).unwrap().len(), 1);
        assert!(db.delete_chat("c").unwrap());
        assert!(db.chats().unwrap().is_empty() && db.message("B").unwrap().is_none());
        assert!(db.reactions("B").unwrap().is_empty());
        assert!(!db.delete_chat("c").unwrap());
    }

    #[test]
    fn pinned_chats_come_first_and_marked_unread_counts_one() {
        let db = Db::in_memory();
        db.add_message(&msg("old", "c1", 1, Status::Sent), None, None).unwrap();
        db.add_message(&msg("new", "c2", 5, Status::Sent), None, None).unwrap();
        assert!(db.set_pinned("c1", true).unwrap());
        let chats = db.chats().unwrap();
        assert_eq!((chats[0].id.as_str(), chats[0].pinned), ("c1", true));
        assert!(db.mark_unread("c2").unwrap());
        assert_eq!(db.chats().unwrap()[1].unread, 1);
        db.mark_read("c2").unwrap();
        assert_eq!(db.chats().unwrap()[1].unread, 0);
    }

    #[test]
    fn drafts_replace_and_go_with_the_chat() {
        let mut db = Db::in_memory();
        db.add_message(&msg("a", "c", 1, Status::Sent), None, None).unwrap();
        db.set_draft("c", "hel", &["/p/1.jpg".into(), "/p/2.mp4".into()]).unwrap();
        assert_eq!(db.draft("c").unwrap(), ("hel".to_string(), vec!["/p/1.jpg".to_string(), "/p/2.mp4".to_string()]));
        db.set_draft("c", "hello", &[]).unwrap();
        assert_eq!(db.draft("c").unwrap(), ("hello".to_string(), vec![]));
        db.set_draft("new", "x", &[]).unwrap();
        assert_eq!(db.draft("new").unwrap().0, "x");
        assert!(db.delete_chat("c").unwrap());
        assert_eq!(db.draft("c").unwrap(), (String::new(), vec![]));
        db.set_draft("new", "", &[]).unwrap();
        assert_eq!(db.draft("new").unwrap(), (String::new(), vec![]));
    }

    #[test]
    fn search_ignores_case_and_unsent() {
        let db = Db::in_memory();
        let add = |id: &str, chat: &str, ts: i64, text: &str| {
            db.add_message(&Message { id, chat_id: chat, sender: "", text, ts, status: Status::Sent, reply_to: None },
                           None, None).unwrap();
        };
        add("1", "c1", 1, "Über den Fluss");
        add("2", "c2", 2, "ganz über");
        add("3", "c1", 3, "nothing");
        add("4", "c1", 4, "über alles");
        db.unsend_message("4").unwrap();
        let ids = |v: Vec<(String, MessageRow)>| v.into_iter().map(|(c, m)| (c, m.id)).collect::<Vec<_>>();
        assert_eq!(ids(db.search("ÜBER", 10).unwrap()),
                   vec![("c2".to_string(), "2".to_string()), ("c1".to_string(), "1".to_string())]);
        assert_eq!(db.search("über", 1).unwrap().len(), 1);
        assert!(db.search("missing", 10).unwrap().is_empty());
    }

    #[test]
    fn muting_is_per_chat() {
        let db = Db::in_memory();
        db.add_message(&msg("a", "c1", 1, Status::Sent), None, None).unwrap();
        db.add_message(&msg("b", "c2", 2, Status::Sent), None, None).unwrap();
        assert!(db.set_muted("c1", true).unwrap());
        assert!(db.muted("c1").unwrap() && !db.muted("c2").unwrap() && !db.muted("nope").unwrap());
        assert!(db.chats().unwrap().iter().any(|c| c.id == "c1" && c.muted));
        assert!(db.set_muted("c1", false).unwrap() && !db.muted("c1").unwrap());
        assert!(!db.set_muted("nope", true).unwrap());
    }

    #[test]
    fn attachments_are_kept_and_go_with_their_message() {
        let mut db = Db::in_memory();
        db.add_message(&msg("A", "c", 1, Status::Received), None, None).unwrap();
        db.add_attachment("A", 1, "b.mov", "video/quicktime", 9, b"i1").unwrap();
        db.add_attachment("A", 0, "a.jpg", "image/jpeg", 5, b"i0").unwrap();
        db.add_attachment("A", 0, "dup.jpg", "image/jpeg", 5, b"x").unwrap();
        let rows = db.attachments("A").unwrap();
        assert_eq!((rows.len(), rows[0].name.as_str(), rows[0].path.as_deref()), (2, "a.jpg", None));
        assert_eq!(db.attachment_info("A", 0).unwrap(),
                   Some((b"i0".to_vec(), "a.jpg".into(), "image/jpeg".into(), "c".into())));
        db.set_attachment_file("A", 0, "/f.jpg", "image/jpeg").unwrap();
        assert_eq!(db.attachments("A").unwrap()[0].path.as_deref(), Some("/f.jpg"));
        assert_eq!(db.message_ids("c").unwrap(), ["A"]);
        db.delete_message("A").unwrap();
        assert!(db.attachments("A").unwrap().is_empty());
        db.add_message(&msg("B", "c", 2, Status::Received), None, None).unwrap();
        db.add_attachment("B", 0, "x", "image/png", 1, b"i").unwrap();
        db.delete_chat("c").unwrap();
        assert!(db.attachments("B").unwrap().is_empty());
    }

    #[test]
    fn old_databases_are_migrated() {
        let path = std::env::temp_dir().join(format!("obd-mig-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Connection::open(&path).unwrap().execute_batch(SCHEMA).unwrap();
        let db = Db::open(&path).unwrap();
        db.add_message(&msg("A", "c", 1, Status::Sent), None, None).unwrap();
        assert!(db.mark_read("c").unwrap());
        drop(db);
        Db::open(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn messages_page_backwards() {
        let db = Db::in_memory();
        for (id, ts) in [("a", 1), ("b", 2), ("c", 3)] {
            db.add_message(&msg(id, "c", ts, Status::Received), None, None).unwrap();
        }
        let page: Vec<_> = db.messages("c", 3, 10).unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(page, ["b", "a"]);
    }
}
