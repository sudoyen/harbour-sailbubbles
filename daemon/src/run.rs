use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use rustpush::{
    APSConnection, APSConnectionResource, ChangeParticipantMessage, ConversationData, DeleteTarget, EditMessage,
    IDSNGMIdentity, IMClient, IconChangeMessage, IndexedMessagePart, Message, MessageInst, MessageParts, MessagePart,
    MessageType, MoveToRecycleBinMessage, NormalMessage, OperatedChat, ReactMessage, ReactMessageType, Reaction,
    RenameMessage, UnsendMessage, MADRID_SERVICE,
};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use zbus::object_server::SignalEmitter;

use crate::api::{self, Api, Outgoing};
use crate::attachments;
use crate::health::{self, Health};
use crate::notify::Notifier;
use crate::db::{self, Db, Event, Status};
use crate::login::{new_uuid, IDENTITY_TAG};
use crate::random;
use crate::state;
use crate::store;

// Only the kind of message is ever logged, never its content or sender.
fn kind(msg: &Message) -> &'static str {
    match msg {
        Message::Message(_) => "message",
        Message::React(_) => "reaction",
        Message::Delivered => "delivered",
        Message::Read => "read",
        Message::Typing(..) => "typing",
        Message::Unsend(_) => "unsend",
        Message::Edit(_) => "edit",
        Message::RenameMessage(_) => "rename",
        Message::ChangeParticipants(_) => "participants",
        Message::IconChange(_) => "icon",
        Message::Error(_) => "error",
        _ => "other",
    }
}

fn is_mine(handle: &str, mine: &[String]) -> bool {
    mine.iter().any(|m| m.eq_ignore_ascii_case(handle))
}

// Stored handles are lowercased; rustpush wants ours exactly as registered.
fn as_sent(participants: Vec<String>, mine: &[String]) -> Vec<String> {
    participants.into_iter()
        .map(|p| mine.iter().find(|m| m.eq_ignore_ascii_case(&p)).cloned().unwrap_or(p))
        .collect()
}

/// Which of our handles the chat is with; the first one if none is.
fn our_handle(participants: &[String], mine: &[String]) -> String {
    mine.iter().find(|m| is_mine(m, participants)).unwrap_or(&mine[0]).clone()
}

fn send_participants(db: &Db, chat_id: &str, mine: &[String]) -> Vec<String> {
    as_sent(db.chat_participants(chat_id).unwrap_or_default(), mine)
}

// rustpush returns once a device of the recipient has the message. The task it
// leaves behind retries their other devices, an offline one for about 20
// minutes, and must not hold up the messages queued after this one.
async fn deliver(client: &IMClient, inst: &mut MessageInst) -> bool {
    client.send(inst).await.is_ok()
}

/// A message of ours unsent while it still waited in the queue.
fn unsent_before_sending(db: &Mutex<Db>, id: &str) -> bool {
    db.lock().unwrap().message(id).ok().flatten().is_some_and(|(m, _)| m.unsent)
}

async fn emit_status(emitter: &SignalEmitter<'_>, db: &Mutex<Db>, id: &str, status: Status) {
    let changed = db.lock().unwrap().set_status(id, status);
    let chat = db.lock().unwrap().chat_of(id);
    match (changed, chat) {
        (Ok(true), Ok(Some(chat))) => {
            let _ = Api::message_status(emitter, &chat, id, status as u32).await;
        }
        (Err(e), _) | (_, Err(e)) => log::error!("updating a message failed: {e}"),
        _ => {}
    }
}

// As OpenBubbles does: the read always goes to our own other devices so they
// clear the chat too; it reaches the other side only for one-to-one chats with
// read receipts on. Groups never get read receipts, also when down to two.
fn read_targets(participants: Vec<String>, me: &str, receipts: bool, group: bool) -> Vec<String> {
    if receipts && !group {
        participants
    } else {
        vec![me.to_string()]
    }
}

fn conversation(db: &Db, chat_id: &str, participants: Vec<String>) -> ConversationData {
    let (name, guid) = db.chat_meta(chat_id).unwrap_or_default();
    // rustpush makes up a GUID per send without one; the same one on every
    // send keeps a group we started one chat on Apple devices
    let guid = guid.or_else(|| {
        let guid = new_uuid().ok()?;
        db.add_guid(chat_id, &guid).ok()?;
        Some(guid)
    });
    ConversationData { participants, cv_name: name, sender_guid: guid, after_guid: None }
}

/// What to send into the chat with, and as which of our handles.
fn chat_conversation(db: &Mutex<Db>, chat_id: &str, mine: &[String]) -> (ConversationData, String) {
    let db = db.lock().unwrap();
    let participants = send_participants(&db, chat_id, mine);
    let me = our_handle(&participants, mine);
    (conversation(&db, chat_id, participants), me)
}

fn reaction_from_kind(kind: &str) -> Option<Reaction> {
    Some(match kind {
        "heart" => Reaction::Heart,
        "like" => Reaction::Like,
        "dislike" => Reaction::Dislike,
        "laugh" => Reaction::Laugh,
        "emphasize" => Reaction::Emphasize,
        "question" => Reaction::Question,
        other => Reaction::Emoji(other.strip_prefix("emoji:")?.to_string()),
    })
}

// Apple addresses the replied-to text as "part:start:length" in UTF-16 units.
fn reply_part(text: &str) -> String {
    format!("0:0:{}", text.encode_utf16().count())
}

// Delete targets name chats by bare addresses; ours keep tel:/mailto: and include us.
fn operated_participants(participants: &[String], me: &str) -> Vec<String> {
    let mut handles: Vec<String> = participants.iter().map(|p| {
        if p.starts_with("tel:") || p.starts_with("mailto:") { p.clone() }
        else if p.contains('@') { format!("mailto:{p}") }
        else { format!("tel:{p}") }
    }).collect();
    handles.push(me.to_string());
    handles
}

fn bare(handle: &str) -> String {
    handle.trim_start_matches("tel:").trim_start_matches("mailto:").to_string()
}

// As OpenBubbles builds it; Apple matches the chat by its group id.
fn operated_chat(participants: &[String], mine: &[String], guid: Option<String>) -> std::io::Result<OperatedChat> {
    let others: Vec<String> = participants.iter().filter(|p| !is_mine(p, mine)).map(|p| bare(p)).collect();
    let name = match others.as_slice() {
        [one] => format!("iMessage;-;{one}"),
        _ => format!("iMessage;+;chat{}", u64::from_be_bytes(random::bytes()?) % 10_000_000_000_000_000),
    };
    Ok(OperatedChat {
        participants: others,
        group_id: match guid { Some(g) => g, None => new_uuid()? },
        guid: name,
        delete_incoming_messages: None,
        was_reported_as_junk: None,
    })
}

async fn send_to(client: &IMClient, me: &str, conv: ConversationData, message: Message, id: Option<String>, what: &str) {
    let mut inst = MessageInst::new(conv, me, message);
    if let Some(id) = id {
        inst.id = id;
    }
    if !deliver(client, &mut inst).await {
        log::error!("sending {what} failed");
    }
}

fn own_devices(me: &str) -> ConversationData {
    ConversationData { participants: vec![me.to_string()], cv_name: None, sender_guid: None, after_guid: None }
}

// Sends a message of ours that is already stored as Sending, and records the outcome.
async fn send_stored(client: &IMClient, mine: &[String], db: &Mutex<Db>, emitter: &SignalEmitter<'_>,
                     id: String, chat_id: String, mut msg: NormalMessage, reply_to: Option<String>) {
    let reply_text = reply_to.as_deref().and_then(|r| db.lock().unwrap().message(r).ok().flatten()).map(|(m, _)| m.text);
    let (conv, me) = chat_conversation(db, &chat_id, mine);
    if let Some(r) = reply_to {
        msg.reply_part = Some(reply_part(&reply_text.unwrap_or_default()));
        msg.reply_guid = Some(r);
    }
    let mut inst = MessageInst::new(conv, &me, Message::Message(msg));
    inst.id = id.clone();
    let status = if deliver(client, &mut inst).await { Status::Sent } else { Status::Failed };
    log::info!("send {}", if status == Status::Sent { "ok" } else { "failed" });
    emit_status(emitter, db, &id, status).await;
}

fn attachment_message(attachment: rustpush::Attachment) -> NormalMessage {
    let mut msg = NormalMessage::new(String::new(), MessageType::IMessage);
    msg.parts = MessageParts(vec![IndexedMessagePart { part: MessagePart::Attachment(attachment), idx: None, ext: None }]);
    msg
}

async fn send_read(client: &IMClient, mine: &[String], db: &Mutex<Db>, chat_id: String) {
    let (conv, me, latest) = {
        let db = db.lock().unwrap();
        let participants = send_participants(&db, &chat_id, mine);
        let me = our_handle(&participants, mine);
        let group = db.group_info(&chat_id).map(|(g, _)| g).unwrap_or(true);
        let targets = read_targets(participants, &me, api::setting(&db, "readReceipts"), group);
        let latest = db.messages(&chat_id, 0, 1).ok().and_then(|m| m.into_iter().find(|m| m.kind.is_empty()));
        (conversation(&db, &chat_id, targets), me, latest)
    };
    let Some(latest) = latest else { return };
    let mut inst = MessageInst::new(conv, &me, Message::Read);
    inst.id = latest.id;
    if !deliver(client, &mut inst).await {
        log::error!("sending a read receipt failed");
    }
}

// Records a group change of ours once it went out.
async fn changed_group(db: &Mutex<Db>, emitter: &SignalEmitter<'_>, chat_id: &str, event_id: &str,
                       event: Event, text: &str, apply: impl FnOnce(&Db) -> rusqlite::Result<()>) {
    let stored = {
        let db = db.lock().unwrap();
        apply(&db).and_then(|_| db.add_event(event_id, chat_id, "", event, text, api::now_ms()))
    };
    if let Err(e) = stored {
        log::error!("storing a group change failed: {e}");
    }
    let _ = Api::chats_changed(emitter).await;
    let _ = Api::message_received(emitter, chat_id, event_id).await;
}

async fn group_change_failed(emitter: &SignalEmitter<'_>, chat_id: &str, action: &str) {
    log::error!("sending a group change failed");
    let _ = Api::action_failed(emitter, chat_id, action).await;
}

async fn send_rename(client: &IMClient, mine: &[String], db: &Mutex<Db>, emitter: &SignalEmitter<'_>, chat_id: String,
                     name: String) {
    let (conv, me) = chat_conversation(db, &chat_id, mine);
    let mut inst = MessageInst::new(conv, &me, Message::RenameMessage(RenameMessage { new_name: name.clone() }));
    if !deliver(client, &mut inst).await {
        return group_change_failed(emitter, &chat_id, "rename").await;
    }
    changed_group(db, emitter, &chat_id, &inst.id, Event::Renamed, &name,
                  |db| db.set_name(&chat_id, Some(name.as_str()).filter(|n| !n.is_empty()))).await;
}

async fn send_participants_change(client: &IMClient, mine: &[String], db: &Mutex<Db>, emitter: &SignalEmitter<'_>,
                                  chat_id: String, participants: Vec<String>, event: Event, handle: String) {
    let action = match event { Event::Added => "add", Event::Left => "leave", _ => "remove" };
    let (conv, me) = chat_conversation(db, &chat_id, mine);
    if event == Event::Added {
        let reachable = client.identity.validate_targets(&[handle.clone()], "com.apple.madrid", &me).await;
        if !reachable.is_ok_and(|r| !r.is_empty()) {
            return group_change_failed(emitter, &chat_id, "addUnreachable").await;
        }
    }
    let version = db.lock().unwrap().group_version(&chat_id).unwrap_or(0) + 1;
    let change = ChangeParticipantMessage { new_participants: as_sent(participants.clone(), mine), group_version: version };
    let mut inst = MessageInst::new(conv, &me, Message::ChangeParticipants(change));
    if !deliver(client, &mut inst).await {
        return group_change_failed(emitter, &chat_id, action).await;
    }
    changed_group(db, emitter, &chat_id, &inst.id, event, &handle, |db| {
        db.set_participants(&chat_id, &participants, api::now_ms())?;
        db.set_group_version(&chat_id, version)
    }).await;
}

async fn send_group_photo(conn: &APSConnection, client: &IMClient, mine: &[String], db: &Mutex<Db>,
                          emitter: &SignalEmitter<'_>, root: &Path, chat_id: String, png: Option<PathBuf>) {
    let version = db.lock().unwrap().group_version(&chat_id).unwrap_or(0) + 1;
    let file = match &png {
        Some(png) => match attachments::upload_group_photo(conn, png).await {
            Ok(file) => Some(file),
            Err(_) => return group_change_failed(emitter, &chat_id, "photo").await,
        },
        None => None,
    };
    let (conv, me) = chat_conversation(db, &chat_id, mine);
    let mut inst = MessageInst::new(conv, &me, Message::IconChange(IconChangeMessage { file, group_version: version }));
    if !deliver(client, &mut inst).await {
        return group_change_failed(emitter, &chat_id, "photo").await;
    }
    let kept = png.and_then(|png| {
        let dest = attachments::group_photo_path(root, &chat_id, version);
        store::ensure_private_dir(dest.parent()?).ok()?;
        std::fs::copy(png, &dest).ok().map(|_| dest)
    });
    attachments::remove_group_photos(root, &chat_id, kept.as_deref());
    let event = if kept.is_some() { Event::Icon } else { Event::IconRemoved };
    changed_group(db, emitter, &chat_id, &inst.id, event, "", |db| {
        db.set_icon(&chat_id, kept.as_deref().map(|p| p.to_string_lossy()).as_deref())?;
        db.set_group_version(&chat_id, version)
    }).await;
}

// One at a time, so a caption sent after its photos also arrives after them.
async fn send_loop(conn: APSConnection, client: Arc<IMClient>, mine: Vec<String>, db: Arc<Mutex<Db>>, root: PathBuf,
                   mut outbox: mpsc::Receiver<Outgoing>, emitter: SignalEmitter<'static>) {
    let me = mine[0].clone();
    // unsent before they went out, so neither they nor their unsends are sent
    let mut never_sent = HashSet::new();
    while let Some(out) = outbox.recv().await {
        match out {
            Outgoing::Text { id, .. } | Outgoing::File { id, .. } if unsent_before_sending(&db, &id) => {
                never_sent.insert(id);
            }
            Outgoing::Text { id, chat_id, text, reply_to } => {
                let msg = NormalMessage::new(text, MessageType::IMessage);
                send_stored(&client, &mine, &db, &emitter, id, chat_id, msg, reply_to).await
            }
            Outgoing::File { id, chat_id, path, name, mime, reply_to } => {
                match attachments::upload(&conn, &path, &name, &mime).await {
                    Ok(a) => send_stored(&client, &mine, &db, &emitter, id, chat_id, attachment_message(a), reply_to).await,
                    Err(_) => {
                        // The error can name the file, so only the fact is logged.
                        log::error!("uploading an attachment failed");
                        emit_status(&emitter, &db, &id, Status::Failed).await;
                    }
                }
            }
            Outgoing::Read { chat_id } => send_read(&client, &mine, &db, chat_id).await,
            Outgoing::React { chat_id, message_id, kind, enable } => {
                let Some(reaction) = reaction_from_kind(&kind) else { continue };
                let to_text = db.lock().unwrap().message(&message_id).ok().flatten().map(|(m, _)| m.text).unwrap_or_default();
                let (conv, sender) = chat_conversation(&db, &chat_id, &mine);
                let react = ReactMessage { to_uuid: message_id, to_part: Some(0), to_text,
                    reaction: ReactMessageType::React { reaction, enable }, embedded_profile: None };
                send_to(&client, &sender, conv, Message::React(react), None, "a reaction").await;
            }
            Outgoing::Edit { chat_id, message_id, text } => {
                let (conv, sender) = chat_conversation(&db, &chat_id, &mine);
                let new_parts = NormalMessage::new(text, MessageType::IMessage).parts;
                let edit = EditMessage { tuuid: message_id, edit_part: 0, new_parts };
                send_to(&client, &sender, conv, Message::Edit(edit), None, "an edit").await;
            }
            Outgoing::Unsend { message_id, .. } if never_sent.remove(&message_id) => {}
            Outgoing::Unsend { chat_id, message_id } => {
                let (conv, sender) = chat_conversation(&db, &chat_id, &mine);
                let unsend = UnsendMessage { tuuid: message_id, edit_part: 0 };
                send_to(&client, &sender, conv, Message::Unsend(unsend), None, "an unsend").await;
            }
            Outgoing::Typing { chat_id, typing } => {
                let (conv, sender) = chat_conversation(&db, &chat_id, &mine);
                send_to(&client, &sender, conv, Message::Typing(typing, None), None, "typing").await;
            }
            Outgoing::DeleteMessage { message_id } => {
                let delete = MoveToRecycleBinMessage { target: DeleteTarget::Messages(vec![message_id]),
                    recoverable_delete_date: api::now_ms() as u64 };
                send_to(&client, &me, own_devices(&me), Message::MoveToRecycleBin(delete), None, "a delete").await;
            }
            Outgoing::DeleteChat { participants, guid } => {
                let Ok(chat) = operated_chat(&participants, &mine, guid) else { continue };
                let delete = MoveToRecycleBinMessage { target: DeleteTarget::Chat(chat),
                    recoverable_delete_date: api::now_ms() as u64 };
                send_to(&client, &me, own_devices(&me), Message::MoveToRecycleBin(delete), None, "a delete").await;
            }
            Outgoing::MarkUnread { message_id } => {
                send_to(&client, &me, own_devices(&me), Message::MarkUnread, Some(message_id), "an unread mark").await;
            }
            Outgoing::Rename { chat_id, name } => send_rename(&client, &mine, &db, &emitter, chat_id, name).await,
            Outgoing::Participants { chat_id, participants, change: (event, handle) } => {
                send_participants_change(&client, &mine, &db, &emitter, chat_id, participants, event, handle).await
            }
            Outgoing::Icon { chat_id, png } => {
                send_group_photo(&conn, &client, &mine, &db, &emitter, &root, chat_id, png).await
            }
        }
    }
}

// As an Apple device does: acknowledge a certified delivery, otherwise send
// the delivered receipt the sender asked for, to the sending device only.
async fn acknowledge(client: &IMClient, mine: &[String], inst: &MessageInst) {
    let result = if let Some(ctx) = &inst.certified_context {
        client.identity.certify_delivery("com.apple.madrid", ctx, inst.send_delivered).await
    } else if inst.send_delivered {
        let (Some(sender), Some(conv)) = (&inst.sender, &inst.conversation) else { return };
        let me = conv.participants.iter().find(|p| is_mine(p, mine)).unwrap_or(&mine[0]);
        let mut receipt = MessageInst::new(ConversationData {
            participants: vec![sender.clone()],
            cv_name: conv.cv_name.clone(),
            sender_guid: conv.sender_guid.clone(),
            after_guid: None,
        }, me, Message::Delivered);
        receipt.id = inst.id.clone();
        receipt.target = inst.target.clone();
        client.send(&mut receipt).await.map(|_| ())
    } else {
        return;
    };
    if result.is_err() {
        log::error!("acknowledging a message failed");
    }
}

// Stored reaction kinds; the app turns them into labels. Stickers and app
// reactions are not shown yet.
fn reaction_kind(reaction: &Reaction) -> Option<String> {
    Some(match reaction {
        Reaction::Heart => "heart".into(),
        Reaction::Like => "like".into(),
        Reaction::Dislike => "dislike".into(),
        Reaction::Laugh => "laugh".into(),
        Reaction::Emphasize => "emphasize".into(),
        Reaction::Question => "question".into(),
        Reaction::Emoji(e) => format!("emoji:{e}"),
        Reaction::Sticker { .. } => return None,
    })
}

fn reaction_verb(kind: &str) -> String {
    match kind {
        "heart" => "Loved".into(),
        "like" => "Liked".into(),
        "dislike" => "Disliked".into(),
        "laugh" => "Laughed at".into(),
        "emphasize" => "Emphasized".into(),
        "question" => "Questioned".into(),
        other => format!("Reacted {}", other.trim_start_matches("emoji:")),
    }
}

async fn react(ctx: &Ctx<'_>, inst: &MessageInst, react: &ReactMessage, from_me: bool) {
    let ReactMessageType::React { reaction, enable } = &react.reaction else { return };
    let Some(kind) = reaction_kind(reaction) else { return };
    let sender = if from_me { String::new() } else { inst.sender.clone().unwrap_or_default().to_lowercase() };
    let target = react.to_uuid.to_uppercase();
    let chat = ctx.db.lock().unwrap().set_reaction(&target, &sender, enable.then_some(kind.as_str()));
    match chat {
        Ok(Some(chat)) => {
            let _ = Api::message_received(ctx.emitter, &chat, &target).await;
            if *enable && !from_me {
                notify(ctx, &chat, &format!("{} \"{}\"", reaction_verb(&kind), react.to_text)).await;
            }
        }
        Ok(None) => {}
        Err(e) => log::error!("storing a reaction failed: {e}"),
    }
}

// A read from our own handle means another of our devices read the chat.
async fn read_elsewhere(ctx: &Ctx<'_>, message_id: &str) {
    let chat = {
        let db = ctx.db.lock().unwrap();
        db.chat_of(message_id).ok().flatten().filter(|chat| db.mark_read(chat).unwrap_or(false))
    };
    let Some(chat) = chat else { return };
    if let Err(e) = ctx.notifier.clear(ctx.emitter.connection(), &chat).await {
        log::error!("closing a notification failed: {e}");
    }
    let _ = Api::chat_read(ctx.emitter, &chat).await;
}

// The daemon marks attachments in text with U+FFFC.
fn notification_text(text: &str) -> String {
    text.replace('\u{fffc}', "[Attachment]").trim().to_string()
}

async fn notify(ctx: &Ctx<'_>, chat: &str, text: &str) {
    let (summary, previews, muted, filter_unknown) = {
        let db = ctx.db.lock().unwrap();
        (api::chat_summary(&db, chat, &ctx.mine_lower), api::setting(&db, "previews"), db.muted(chat).unwrap_or(false),
         api::setting(&db, "filterUnknown"))
    };
    let Ok(Some((title, unread, unknown))) = summary else { return };
    // as OpenBubbles: filtered unknown senders are listed apart, silently
    if muted || (filter_unknown && unknown) {
        return;
    }
    let body = if previews { notification_text(text) } else { "New message".to_string() };
    if let Err(e) = ctx.notifier.message(ctx.emitter.connection(), chat, &title, &body, unread).await {
        log::error!("posting a notification failed: {e}");
    }
}

struct Ctx<'a> {
    conn: &'a APSConnection,
    db: &'a Mutex<Db>,
    emitter: &'a SignalEmitter<'a>,
    notifier: &'a Notifier,
    me_lower: String,
    mine_lower: Vec<String>,
    downloads: &'a mpsc::Sender<(String, u32)>,
    attachments_root: &'a Path,
}

/// Stores the message's attachments; returns those small enough to fetch now.
fn record_attachments(db: &Db, message_id: &str, msg: &NormalMessage) -> Vec<u32> {
    let mut fetch = vec![];
    for (idx, part) in msg.parts.0.iter().enumerate() {
        let MessagePart::Attachment(a) = &part.part else { continue };
        let mut info = Vec::new();
        if let Err(e) = plist::to_writer_binary(&mut info, a) {
            log::error!("keeping an attachment failed: {e}");
            continue;
        }
        let size = a.get_size() as i64;
        match db.add_attachment(message_id, idx as u32, &a.name, &a.mime, size, &info) {
            Ok(()) if size <= attachments::AUTO_DOWNLOAD_MAX => fetch.push(idx as u32),
            Ok(()) => {}
            Err(e) => log::error!("storing an attachment failed: {e}"),
        }
    }
    fetch
}

async fn download_loop(conn: APSConnection, db: Arc<Mutex<Db>>, root: PathBuf,
                       mut queue: mpsc::Receiver<(String, u32)>, emitter: SignalEmitter<'static>) {
    while let Some((message_id, idx)) = queue.recv().await {
        let Ok(Some((info, name, mime, chat))) = db.lock().unwrap().attachment_info(&message_id, idx) else { continue };
        let dest = attachments::path_for(&root, &message_id, idx, &name);
        if let Err(e) = attachments::download(&conn, &info, &dest).await {
            log::error!("downloading an attachment failed: {e}");
            continue;
        }
        let (file, mime) = attachments::displayable(dest, &mime, &name);
        if let Err(e) = db.lock().unwrap().set_attachment_file(&message_id, idx, &file.to_string_lossy(), &mime) {
            log::error!("storing an attachment path failed: {e}");
            continue;
        }
        let _ = Api::message_received(&emitter, &chat, &message_id).await;
    }
}

/// The chat a received message is for, made when new: Apple names a group by
/// its sender GUID, a chat without one by its participants.
fn chat_for(db: &Db, conv: &ConversationData) -> rusqlite::Result<String> {
    let chat = match db.find_chat(conv.sender_guid.as_deref(), &conv.participants)? {
        Some(chat) => chat,
        None => db.create_chat(&conv.participants, conv.cv_name.as_deref(), false)?,
    };
    if let Some(guid) = &conv.sender_guid {
        db.add_guid(&chat, guid)?;
    }
    Ok(chat)
}

fn lowercase(handles: &[String]) -> Vec<String> {
    handles.iter().map(|h| h.to_lowercase()).collect()
}

/// Stores who joined and left between `old` and `new` as events of `actor`
/// (stored as `sender`); returns whether any was new.
#[allow(clippy::too_many_arguments)]
fn record_members(db: &Db, chat: &str, event_id: &str, actor: &str, sender: &str, old: &[String], new: &[String],
                  ts: i64) -> rusqlite::Result<bool> {
    let joined = new.iter().filter(|p| !old.contains(p)).map(|p| (Event::Added, p));
    let gone = old.iter().filter(|p| !new.contains(p))
        .map(|p| (if p == actor { Event::Left } else { Event::Removed }, p));
    let mut any = false;
    for (n, (event, handle)) in joined.chain(gone).enumerate() {
        let id = if n == 0 { event_id.to_string() } else { format!("{event_id}/{n}") };
        any |= db.add_event(&id, chat, sender, event, handle, ts)?;
    }
    Ok(any)
}

// Apple devices send the group's members and name with every message; follow
// them, as OpenBubbles does, in case a change itself never reached us. A
// message older than the last change knows only the members before it.
fn follow_group(db: &Db, chat: &str, conv: &ConversationData, inst: &MessageInst, actor: &str, sender: &str)
    -> rusqlite::Result<bool>
{
    let (old, new) = (db.chat_participants(chat)?, lowercase(&conv.participants));
    let (group, members_ts) = db.group_info(chat)?;
    let ts = inst.sent_timestamp as i64;
    let mut changed = false;
    if group && new.len() > 2 && ts > members_ts && db::chat_id(&old) != db::chat_id(&new) {
        changed |= record_members(db, chat, &format!("{}/members", inst.id), actor, sender, &old, &new, ts)?;
        db.set_participants(chat, &new, ts)?;
    }
    if let Some(name) = conv.cv_name.as_deref().filter(|n| !n.is_empty()) {
        if db.chat_meta(chat)?.0.as_deref() != Some(name) {
            db.set_name(chat, Some(name))?;
            changed = true;
        }
    }
    Ok(changed)
}

fn actor_and_sender(inst: &MessageInst, from_me: bool, me: &str) -> (String, String) {
    if from_me {
        (me.to_string(), String::new())
    } else {
        let sender = inst.sender.clone().unwrap_or_default().to_lowercase();
        (sender.clone(), sender)
    }
}

async fn group_event_stored(ctx: &Ctx<'_>, stored: rusqlite::Result<(String, bool)>, event_id: &str) {
    match stored {
        Ok((chat, true)) => {
            let _ = Api::chats_changed(ctx.emitter).await;
            let _ = Api::message_received(ctx.emitter, &chat, event_id).await;
        }
        Ok(_) => {}
        Err(e) => log::error!("storing a group change failed: {e}"),
    }
}

async fn renamed(ctx: &Ctx<'_>, inst: &MessageInst, rename: &RenameMessage, from_me: bool) {
    let Some(conv) = &inst.conversation else { return };
    let (_, sender) = actor_and_sender(inst, from_me, &ctx.me_lower);
    let name = rename.new_name.trim();
    let stored = {
        let db = ctx.db.lock().unwrap();
        chat_for(&db, conv).and_then(|chat| {
            db.set_name(&chat, Some(name).filter(|n| !n.is_empty()))?;
            let added = db.add_event(&inst.id, &chat, &sender, Event::Renamed, name, inst.sent_timestamp as i64)?;
            Ok((chat, added))
        })
    };
    group_event_stored(ctx, stored, &inst.id).await;
}

async fn members_changed(ctx: &Ctx<'_>, inst: &MessageInst, change: &ChangeParticipantMessage, from_me: bool) {
    let Some(conv) = &inst.conversation else { return };
    let (actor, sender) = actor_and_sender(inst, from_me, &ctx.me_lower);
    let new = lowercase(&change.new_participants);
    let stored = {
        let db = ctx.db.lock().unwrap();
        (|| {
            // the chat goes by the members before the change, or already after it
            let chat = match db.find_group(conv.sender_guid.as_deref(), &conv.participants)? {
                Some(chat) => chat,
                None => match db.find_group(None, &new)? {
                    Some(chat) => chat,
                    None => db.create_chat(&new, conv.cv_name.as_deref(), true)?,
                },
            };
            if let Some(guid) = &conv.sender_guid {
                db.add_guid(&chat, guid)?;
            }
            // a change older than the one we have, sent again or late, is stale
            if change.group_version < db.group_version(&chat)? {
                return Ok((chat, false));
            }
            let old = db.chat_participants(&chat)?;
            let ts = inst.sent_timestamp as i64;
            let added = record_members(&db, &chat, &inst.id, &actor, &sender, &old, &new, ts)?;
            db.set_participants(&chat, &new, ts)?;
            db.set_group_version(&chat, change.group_version)?;
            Ok((chat, added))
        })()
    };
    group_event_stored(ctx, stored, &inst.id).await;
}

async fn icon_changed(ctx: &Ctx<'_>, inst: &MessageInst, change: &IconChangeMessage, from_me: bool) {
    let Some(conv) = &inst.conversation else { return };
    let (_, sender) = actor_and_sender(inst, from_me, &ctx.me_lower);
    let found = {
        let db = ctx.db.lock().unwrap();
        chat_for(&db, conv).and_then(|chat| Ok((db.group_version(&chat)?, chat)))
    };
    let (version, chat) = match found {
        Ok(found) => found,
        Err(e) => return log::error!("finding a chat failed: {e}"),
    };
    // as OpenBubbles: an older change than the one we have is stale
    if change.group_version <= version {
        return;
    }
    let path = match &change.file {
        Some(file) => {
            let dest = attachments::group_photo_path(ctx.attachments_root, &chat, change.group_version);
            if let Err(e) = attachments::download_group_photo(ctx.conn, file, &dest).await {
                return log::error!("downloading a group photo failed: {e}");
            }
            Some(dest)
        }
        None => None,
    };
    attachments::remove_group_photos(ctx.attachments_root, &chat, path.as_deref());
    let event = if path.is_some() { Event::Icon } else { Event::IconRemoved };
    let stored = {
        let db = ctx.db.lock().unwrap();
        (|| {
            db.set_icon(&chat, path.as_deref().map(|p| p.to_string_lossy()).as_deref())?;
            db.set_group_version(&chat, change.group_version)?;
            Ok((chat.clone(), db.add_event(&inst.id, &chat, &sender, event, "", inst.sent_timestamp as i64)?))
        })()
    };
    group_event_stored(ctx, stored, &inst.id).await;
}

// As OpenBubbles' "unarchive on new message", off unless set.
fn unarchive(db: &Mutex<Db>, chat: &str) -> bool {
    let db = db.lock().unwrap();
    api::setting(&db, "unarchiveOnNewMessage") && db.set_archived_if(chat, true, false).unwrap_or(false)
}

async fn store(ctx: &Ctx<'_>, inst: &MessageInst, from_me: bool) {
    let (emitter, db) = (ctx.emitter, ctx.db);
    let (Message::Message(msg), Some(conv)) = (&inst.message, &inst.conversation) else { return };
    let (actor, sender) = actor_and_sender(inst, from_me, &ctx.me_lower);
    let found = {
        let db = db.lock().unwrap();
        chat_for(&db, conv).and_then(|chat| Ok((follow_group(&db, &chat, conv, inst, &actor, &sender)?, chat)))
    };
    let (group_changed, chat) = match found {
        Ok(found) => found,
        Err(e) => return log::error!("finding a chat failed: {e}"),
    };
    if group_changed {
        let _ = Api::chats_changed(emitter).await;
    }
    let reply_to = msg.reply_guid.as_ref().map(|r| r.to_uppercase());
    let added = db.lock().unwrap().add_message(&db::Message {
        id: &inst.id,
        chat_id: &chat,
        sender: &sender,
        text: &msg.parts.raw_text(),
        ts: inst.sent_timestamp as i64,
        status: if from_me { Status::Sent } else { Status::Received },
        reply_to: reply_to.as_deref(),
    }, conv.cv_name.as_deref(), conv.sender_guid.as_deref());
    match added {
        Ok(true) => {
            let fetch = record_attachments(&db.lock().unwrap(), &inst.id, msg);
            for idx in fetch {
                let _ = ctx.downloads.send((inst.id.clone(), idx)).await;
            }
            if !from_me && unarchive(db, &chat) {
                let _ = Api::chats_changed(emitter).await;
            }
            let _ = Api::message_received(emitter, &chat, &inst.id).await;
            if !from_me {
                notify(ctx, &chat, &msg.parts.raw_text()).await;
            }
        }
        Ok(false) => {}
        Err(e) => log::error!("storing a message failed: {e}"),
    }
}

async fn changed(ctx: &Ctx<'_>, chat: rusqlite::Result<Option<String>>, message_id: &str) {
    match chat {
        Ok(Some(chat)) => {
            let _ = Api::message_received(ctx.emitter, &chat, message_id).await;
        }
        Ok(None) => {}
        Err(e) => log::error!("updating a message failed: {e}"),
    }
}

async fn deleted_elsewhere(ctx: &Ctx<'_>, delete: &MoveToRecycleBinMessage) {
    match &delete.target {
        DeleteTarget::Messages(ids) => {
            for id in ids {
                let id = id.to_uppercase();
                attachments::remove(ctx.attachments_root, &id);
                let chat = ctx.db.lock().unwrap().delete_message(&id);
                changed(ctx, chat, &id).await;
            }
        }
        DeleteTarget::Chat(op) => {
            let participants = operated_participants(&op.participants, &ctx.me_lower);
            let Ok(Some(chat)) = ctx.db.lock().unwrap().find_chat(Some(&op.group_id), &participants) else { return };
            for id in ctx.db.lock().unwrap().message_ids(&chat).unwrap_or_default() {
                attachments::remove(ctx.attachments_root, &id);
            }
            if ctx.db.lock().unwrap().delete_chat(&chat).unwrap_or(false) {
                let _ = ctx.notifier.clear(ctx.emitter.connection(), &chat).await;
                let _ = Api::chats_changed(ctx.emitter).await;
            }
        }
    }
}

async fn on_received(client: &IMClient, mine: &[String], ctx: &Ctx<'_>, inst: MessageInst) {
    let from_me = inst.sender.as_deref().is_some_and(|s| is_mine(s, mine));
    match &inst.message {
        Message::Message(_) => store(ctx, &inst, from_me).await,
        Message::React(r) => react(ctx, &inst, r, from_me).await,
        Message::Delivered => emit_status(ctx.emitter, ctx.db, &inst.id, Status::Delivered).await,
        Message::Read if from_me => read_elsewhere(ctx, &inst.id).await,
        Message::Read => emit_status(ctx.emitter, ctx.db, &inst.id, Status::Read).await,
        Message::Edit(e) => {
            let id = e.tuuid.to_uppercase();
            let chat = ctx.db.lock().unwrap().edit_message(&id, &e.new_parts.raw_text(), inst.sent_timestamp as i64);
            changed(ctx, chat, &id).await;
        }
        Message::Unsend(u) => {
            let id = u.tuuid.to_uppercase();
            let chat = ctx.db.lock().unwrap().unsend_message(&id);
            changed(ctx, chat, &id).await;
        }
        Message::Typing(typing, _) if !from_me => {
            let chat = inst.conversation.as_ref()
                .and_then(|conv| ctx.db.lock().unwrap().find_chat(conv.sender_guid.as_deref(), &conv.participants).ok().flatten());
            if let Some(chat) = chat {
                let _ = Api::typing_changed(ctx.emitter, &chat, *typing).await;
            }
        }
        Message::RenameMessage(r) => renamed(ctx, &inst, r, from_me).await,
        Message::ChangeParticipants(c) => members_changed(ctx, &inst, c, from_me).await,
        Message::IconChange(i) => icon_changed(ctx, &inst, i, from_me).await,
        Message::MoveToRecycleBin(delete) if from_me => deleted_elsewhere(ctx, delete).await,
        Message::MarkUnread if from_me => {
            let marked = {
                let db = ctx.db.lock().unwrap();
                db.chat_of(&inst.id).ok().flatten().is_some_and(|chat| db.mark_unread(&chat).unwrap_or(false))
            };
            if marked {
                let _ = Api::chats_changed(ctx.emitter).await;
            }
        }
        _ => {}
    }
    if !from_me {
        acknowledge(client, mine, &inst).await;
    }
}

// Follows the push connection and the registration, which rustpush keeps up
// by itself, so the app can show them and a needed sign-in is not missed.
async fn watch_health(conn: APSConnection, client: Arc<IMClient>, health: Arc<Mutex<Health>>,
                      notifier: Arc<Notifier>, emitter: SignalEmitter<'static>) {
    let mut push = conn.resource_state.subscribe();
    let mut registration = client.identity.resource_state.subscribe();
    loop {
        let (p, r) = (health::push_state(&push.borrow_and_update()),
                      health::registration_state(&registration.borrow_and_update()));
        let renew_in_s = if r == "ok" { client.identity.calculate_rereg_time_s().await } else { 0 };
        let next = Health { push: p, registration: r, renew_at_ms: if renew_in_s > 0 { api::now_ms() + renew_in_s * 1000 } else { 0 } };
        let before = std::mem::replace(&mut *health.lock().unwrap(), next.clone());
        if before.push != next.push || before.registration != next.registration {
            log::info!("push {p}, registration {r}");
            if r == "signin" && before.registration != "signin" {
                let _ = notifier.alert(emitter.connection(), "iMessage needs you to sign in again",
                    "Open SailBubbles, then Settings, Sign in again").await;
            }
        }
        if before != next {
            let _ = Api::status_changed(&emitter).await;
        }
        tokio::select! {
            r = push.changed() => if r.is_err() { break },
            r = registration.changed() => if r.is_err() { break },
        }
    }
}

pub async fn run(dir: &Path) -> Result<()> {
    let Some(saved) = state::load(dir).context("reading state")? else {
        bail!("no saved login");
    };
    // Claiming the bus name first keeps a second daemon from also connecting
    // to Apple with the same push identity.
    let bus = zbus::connection::Builder::session()?.name(api::NAME)?.build().await
        .context("another sailbubbles-daemon is running, or there is no session bus")?;

    let relay = Arc::new(saved.relay.clone());
    let (conn, err) = APSConnectionResource::new(relay.clone(), Some(saved.push.clone())).await;
    if let Some(e) = err {
        return Err(e).context("push connection failed");
    }
    log::info!("push connected");

    let identity = IDSNGMIdentity::restore(&saved.identity, IDENTITY_TAG)?;
    let users = saved.users.clone();
    let shared = Arc::new(Mutex::new(saved));
    let state_dir = dir.to_path_buf();
    let on_keys = Box::new(move |users| {
        let mut saved = shared.lock().unwrap();
        saved.users = users;
        if let Err(e) = state::save(&state_dir, &saved) {
            log::error!("saving state failed: {e}");
        }
    });
    let client = Arc::new(IMClient::new(conn.clone(), users, identity, &[&MADRID_SERVICE],
                                        dir.join("id_cache.plist"), relay, on_keys).await);
    let mine = client.identity.get_handles().await;
    let me = mine.first().context("no registered iMessage handle")?.clone();

    let db = Arc::new(Mutex::new(Db::open(&dir.join("messages.db")).context("opening messages")?));
    let (outbox_tx, outbox_rx) = mpsc::channel(32);
    let notifier = Arc::new(Notifier::default());
    let health = Arc::new(Mutex::new(Health::default()));
    let attachments_root = attachments::root()?;
    let (downloads_tx, downloads_rx) = mpsc::channel(256);
    bus.object_server().at(api::PATH, Api {
        db: db.clone(), me: me.to_lowercase(), mine: mine.iter().map(|h| h.to_lowercase()).collect(), outbox: outbox_tx, notifier: notifier.clone(), health: health.clone(),
        downloads: downloads_tx.clone(), attachments_root: attachments_root.clone(), home: attachments::home()?,
        state_dir: dir.to_path_buf(),
    }).await?;
    let emitter = SignalEmitter::new(&bus, api::PATH)?.into_owned();
    tokio::spawn(watch_health(conn.clone(), client.clone(), health, notifier.clone(), emitter.clone()));
    tokio::spawn(download_loop(conn.clone(), db.clone(), attachments_root.clone(), downloads_rx, emitter.clone()));
    let ctx = Ctx { conn: &conn, db: &db, emitter: &emitter, notifier: &notifier, me_lower: me.to_lowercase(),
                    mine_lower: mine.iter().map(|h| h.to_lowercase()).collect(),
                    downloads: &downloads_tx, attachments_root: &attachments_root };
    tokio::spawn(send_loop(conn.clone(), client.clone(), mine.clone(), db.clone(), attachments_root.clone(), outbox_rx,
                           emitter.clone()));
    log::info!("listening");

    let mut msgs = conn.messages_cont.subscribe();
    loop {
        let msg = match msgs.recv().await {
            Ok(msg) => msg,
            Err(RecvError::Lagged(n)) => {
                log::error!("dropped {n} push messages");
                continue;
            }
            Err(RecvError::Closed) => bail!("push connection closed"),
        };
        match client.handle(msg).await {
            Ok(Some(inst)) => {
                log::info!("received {}", kind(&inst.message));
                on_received(&client, &mine, &ctx, inst).await;
            }
            Ok(None) => {}
            // PushError text can carry handles, so it is not logged.
            Err(_) => log::error!("a push message could not be handled"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_never_contains_text() {
        let msg = Message::Message(NormalMessage::new("secret words".into(), MessageType::IMessage));
        assert_eq!(kind(&msg), "message");
    }

    #[test]
    fn receipts_are_named() {
        assert_eq!(kind(&Message::Delivered), "delivered");
        assert_eq!(kind(&Message::Read), "read");
        assert_eq!(kind(&Message::Typing(true, None)), "typing");
    }

    #[test]
    fn own_handle_keeps_its_case_for_sending() {
        let db = Db::in_memory();
        let chat = db.create_chat(&["mailto:Me@Example.org".into(), "tel:+1555".into()], None, false).unwrap();
        let mine = ["mailto:Alias@Example.org".to_string(), "mailto:Me@Example.org".to_string()];
        assert_eq!(send_participants(&db, &chat, &mine), ["mailto:Me@Example.org", "tel:+1555"]);
        assert_eq!(our_handle(&db.chat_participants(&chat).unwrap(), &mine), "mailto:Me@Example.org");
        assert_eq!(our_handle(&["tel:+1555".to_string()], &mine), "mailto:Alias@Example.org");
    }

    #[test]
    fn a_message_unsent_in_the_queue_is_not_sent() {
        let db = Mutex::new(Db::in_memory());
        db.lock().unwrap().add_message(&db::Message { id: "A", chat_id: "c", sender: "", text: "oops", ts: 1,
                                                      status: Status::Sending, reply_to: None }, None, None).unwrap();
        assert!(!unsent_before_sending(&db, "A"));
        db.lock().unwrap().unsend_message("A").unwrap();
        assert!(unsent_before_sending(&db, "A"));
        assert!(!unsent_before_sending(&db, "unknown"));
    }

    #[test]
    fn member_changes_become_events() {
        let db = Db::in_memory();
        let p = |l: &[&str]| l.iter().map(|h| h.to_string()).collect::<Vec<_>>();
        let (old, new) = (p(&["me", "tel:+1", "tel:+2", "tel:+3"]), p(&["me", "tel:+1", "tel:+4"]));
        let chat = db.create_chat(&old, None, false).unwrap();
        assert!(record_members(&db, &chat, "E", "tel:+3", "tel:+3", &old, &new, 5).unwrap());
        let mut events: Vec<_> = db.messages(&chat, 0, 10).unwrap().into_iter().map(|m| (m.id, m.kind, m.text)).collect();
        events.sort();
        assert_eq!(events, [("E".into(), "added".into(), "tel:+4".into()),
                            ("E/1".into(), "removed".into(), "tel:+2".into()),
                            ("E/2".into(), "left".into(), "tel:+3".into())]);
        assert!(!record_members(&db, &chat, "E", "tel:+3", "tel:+3", &old, &new, 5).unwrap());
    }

    #[test]
    fn group_members_and_name_follow_incoming_messages() {
        let db = Db::in_memory();
        let p = |l: &[&str]| l.iter().map(|h| h.to_string()).collect::<Vec<_>>();
        let chat = db.create_chat(&p(&["me", "tel:+1", "tel:+2"]), None, false).unwrap();
        let conv = ConversationData { participants: p(&["me", "tel:+1", "TEL:+3"]), cv_name: Some("Climbing".into()),
                                      sender_guid: None, after_guid: None };
        let mut inst = MessageInst::new(conv.clone(), "tel:+1", Message::Read);
        inst.sent_timestamp = 10;
        assert!(follow_group(&db, &chat, &conv, &inst, "tel:+1", "tel:+1").unwrap());
        assert_eq!(db.chat_participants(&chat).unwrap(), ["me", "tel:+1", "tel:+3"]);
        assert_eq!(db.chat_meta(&chat).unwrap().0.as_deref(), Some("Climbing"));
        assert!(!follow_group(&db, &chat, &conv, &inst, "tel:+1", "tel:+1").unwrap());

        // a message sent before that change, delivered after it, changes nothing back
        let late = ConversationData { participants: p(&["me", "tel:+1", "tel:+2"]), ..conv.clone() };
        let mut old = MessageInst::new(late.clone(), "tel:+1", Message::Read);
        old.sent_timestamp = 5;
        assert!(!follow_group(&db, &chat, &late, &old, "tel:+1", "tel:+1").unwrap());
        assert_eq!(db.chat_participants(&chat).unwrap(), ["me", "tel:+1", "tel:+3"]);

        let one = db.create_chat(&p(&["me", "tel:+1"]), None, false).unwrap();
        let conv = ConversationData { participants: p(&["me", "tel:+1"]), cv_name: None, sender_guid: None, after_guid: None };
        assert!(!follow_group(&db, &one, &conv, &inst, "tel:+1", "tel:+1").unwrap());
    }

    #[test]
    fn received_messages_find_their_chat() {
        let db = Db::in_memory();
        let p = |l: &[&str]| l.iter().map(|h| h.to_string()).collect::<Vec<_>>();
        let conv = |people: &[&str], guid: Option<&str>| ConversationData {
            participants: p(people), cv_name: None, sender_guid: guid.map(str::to_string), after_guid: None };
        let chat = chat_for(&db, &conv(&["me", "tel:+1", "tel:+2"], Some("G"))).unwrap();
        assert_eq!(chat_for(&db, &conv(&["me", "tel:+1", "tel:+5"], Some("G"))).unwrap(), chat);
        assert_eq!(chat_for(&db, &conv(&["tel:+2", "me", "TEL:+1"], None)).unwrap(), chat);
        assert_ne!(chat_for(&db, &conv(&["me", "tel:+1"], None)).unwrap(), chat);
    }

    #[test]
    fn reactions_map_to_stored_kinds() {
        assert_eq!(reaction_kind(&Reaction::Heart).as_deref(), Some("heart"));
        assert_eq!(reaction_kind(&Reaction::Emoji("x".into())).as_deref(), Some("emoji:x"));
        assert_eq!(reaction_kind(&Reaction::Sticker { spec: None, body: rustpush::MessageParts(vec![]) }), None);
        assert_eq!(reaction_verb("laugh"), "Laughed at");
        assert_eq!(reaction_verb("emoji:x"), "Reacted x");
    }

    #[test]
    fn reaction_kinds_round_trip() {
        for kind in api::REACTIONS.iter().copied().chain(["emoji:x"]) {
            assert_eq!(reaction_kind(&reaction_from_kind(kind).unwrap()).as_deref(), Some(kind));
        }
        assert!(reaction_from_kind("bogus").is_none());
    }

    #[test]
    fn reply_part_counts_utf16() {
        assert_eq!(reply_part("hi"), "0:0:2");
        assert_eq!(reply_part("\u{1F600}"), "0:0:2");
    }

    #[test]
    fn operated_chats_map_back_to_our_ids() {
        let me = "mailto:me@x.org";
        let participants: Vec<String> = vec![me.into(), "mailto:a@b.org".into(), "tel:+1".into()];
        let chat = db::chat_id(&participants);
        assert_eq!(db::chat_id(&operated_participants(&["+1".into(), "a@b.org".into()], me)), chat);
        let op = operated_chat(&participants, &[me.to_string()], Some("G".into())).unwrap();
        assert_eq!(op.group_id, "G");
        assert!(op.guid.starts_with("iMessage;+;chat"));
        assert_eq!(db::chat_id(&operated_participants(&op.participants, me)), chat);
        let one = operated_chat(&[me.into(), "tel:+1".into()], &[me.to_string()], None).unwrap();
        assert_eq!(one.guid, "iMessage;-;+1");
    }

    #[test]
    fn attachments_go_alone_as_one_part() {
        let a = rustpush::Attachment { a_type: rustpush::AttachmentType::Inline(vec![1]), part: 0,
            uti_type: "public.jpeg".into(), mime: "image/jpeg".into(), name: "a.jpg".into(), iris: false };
        let msg = attachment_message(a);
        assert_eq!(msg.parts.0.len(), 1);
        assert!(matches!(&msg.parts.0[0].part, MessagePart::Attachment(a) if a.name == "a.jpg"));
    }

    #[test]
    fn notification_text_names_attachments() {
        assert_eq!(notification_text("\u{fffc} look"), "[Attachment] look");
    }

    #[test]
    fn read_receipts_only_reach_one_to_one_chats_when_on() {
        let me = "mailto:me@x.org";
        let one = vec![me.to_string(), "tel:+1".to_string()];
        let group = vec![me.to_string(), "tel:+1".to_string(), "tel:+2".to_string()];
        assert_eq!(read_targets(one.clone(), me, true, false), one);
        assert_eq!(read_targets(one.clone(), me, false, false), [me]);
        assert_eq!(read_targets(group, me, true, true), [me]);
        assert_eq!(read_targets(one, me, true, true), [me], "a group down to two is still a group");
    }

    #[test]
    fn mine_ignores_case() {
        let mine = vec!["mailto:Me@Example.org".to_string()];
        assert!(is_mine("mailto:me@example.org", &mine));
        assert!(!is_mine("tel:+1555", &mine));
    }
}
