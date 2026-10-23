use std::collections::HashMap;
use std::sync::Mutex;

use zbus::zvariant::Value;

const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
// shipped in /usr/share/lipstick/notificationcategories by the daemon RPM
const CATEGORY: &str = "x-sailbubbles.message";
const APP_SERVICE: &str = "org.sailbubbles.harbour-sailbubbles";

fn base64(data: &[u8]) -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            out.push(if i <= chunk.len() { ABC[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

// Sailfish notification actions carry their arguments as QVariants written
// with QDataStream: type id (QString = 10), null flag, then UTF-16BE text.
fn qvariant_string(s: &str) -> Vec<u8> {
    let utf16: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let mut v = 10u32.to_be_bytes().to_vec();
    v.push(0);
    v.extend((utf16.len() as u32).to_be_bytes());
    v.extend(utf16);
    v
}

fn open_chat_action(chat_id: &str) -> String {
    format!("{APP_SERVICE} / org.sailbubbles.App openChat {}", base64(&qvariant_string(chat_id)))
}

/// One notification per chat, replaced on each new message.
#[derive(Default)]
pub struct Notifier {
    ids: Mutex<HashMap<String, u32>>,
}

impl Notifier {
    pub async fn message(&self, bus: &zbus::Connection, chat_id: &str, title: &str, body: &str, unread: u32)
        -> zbus::Result<()>
    {
        let replaces = self.ids.lock().unwrap().get(chat_id).copied().unwrap_or(0);
        let action = open_chat_action(chat_id);
        let hints: HashMap<&str, Value> = HashMap::from([
            ("category", Value::from(CATEGORY)),
            ("x-nemo-preview-summary", Value::from(title)),
            ("x-nemo-preview-body", Value::from(body)),
            ("x-nemo-item-count", Value::from(unread as i32)),
            ("x-nemo-remote-action-default", Value::from(action.as_str())),
        ]);
        let reply = bus.call_method(Some(SERVICE), PATH, Some(SERVICE), "Notify",
            &("SailBubbles", replaces, "", title, body, vec!["default", ""], hints, -1i32)).await?;
        let id: u32 = reply.body().deserialize()?;
        self.ids.lock().unwrap().insert(chat_id.to_string(), id);
        Ok(())
    }

    /// A one-off notice that opens nothing when tapped.
    pub async fn alert(&self, bus: &zbus::Connection, title: &str, body: &str) -> zbus::Result<()> {
        let hints: HashMap<&str, Value> = HashMap::from([
            ("category", Value::from(CATEGORY)),
            ("x-nemo-preview-summary", Value::from(title)),
            ("x-nemo-preview-body", Value::from(body)),
        ]);
        bus.call_method(Some(SERVICE), PATH, Some(SERVICE), "Notify",
            &("SailBubbles", 0u32, "", title, body, Vec::<&str>::new(), hints, -1i32)).await?;
        Ok(())
    }

    pub async fn clear(&self, bus: &zbus::Connection, chat_id: &str) -> zbus::Result<()> {
        let Some(id) = self.ids.lock().unwrap().remove(chat_id) else { return Ok(()) };
        bus.call_method(Some(SERVICE), PATH, Some(SERVICE), "CloseNotification", &(id,)).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::net::UnixStream;
    use zbus::{connection::Builder, interface};

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn qvariant_string_matches_qdatastream() {
        // QDataStream << QVariant(QString("ab")), Qt 5
        assert_eq!(qvariant_string("ab"), [0, 0, 0, 10, 0, 0, 0, 0, 4, 0, 0x61, 0, 0x62]);
    }

    #[test]
    fn action_calls_the_app() {
        assert_eq!(open_chat_action("ab"),
                   "org.sailbubbles.harbour-sailbubbles / org.sailbubbles.App openChat AAAACgAAAAAEAGEAYg==");
    }

    #[derive(Default)]
    struct Fake {
        calls: Arc<Mutex<Vec<(u32, String, String)>>>,
        closed: Arc<Mutex<Vec<u32>>>,
    }

    #[interface(name = "org.freedesktop.Notifications")]
    impl Fake {
        #[allow(clippy::too_many_arguments)]
        fn notify(&self, _app: String, replaces: u32, _icon: String, summary: String, body: String,
                  _actions: Vec<String>, hints: HashMap<String, zbus::zvariant::OwnedValue>, _timeout: i32) -> u32 {
            assert_eq!(hints["category"], Value::from(CATEGORY).try_into().unwrap());
            self.calls.lock().unwrap().push((replaces, summary, body));
            if replaces == 0 { 7 } else { replaces }
        }

        fn close_notification(&self, id: u32) {
            self.closed.lock().unwrap().push(id);
        }
    }

    #[tokio::test]
    async fn chats_reuse_their_notification_and_clear_it() {
        let fake = Fake::default();
        let (calls, closed) = (fake.calls.clone(), fake.closed.clone());
        let (a, b) = UnixStream::pair().unwrap();
        let server = Builder::unix_stream(a).server(zbus::Guid::generate()).unwrap().p2p()
            .serve_at(PATH, fake).unwrap().build();
        let client = Builder::unix_stream(b).p2p().build();
        let (server, client) = tokio::try_join!(server, client).unwrap();
        std::mem::forget(server);

        let n = Notifier::default();
        n.message(&client, "c1", "Anna", "hi", 1).await.unwrap();
        n.message(&client, "c1", "Anna", "again", 2).await.unwrap();
        n.clear(&client, "c1").await.unwrap();
        n.clear(&client, "c1").await.unwrap();

        assert_eq!(*calls.lock().unwrap(), [(0, "Anna".into(), "hi".into()), (7, "Anna".into(), "again".into())]);
        assert_eq!(*closed.lock().unwrap(), [7]);
    }
}
