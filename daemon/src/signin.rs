use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use tokio::sync::mpsc;
use zbus::{fdo, interface, object_server::SignalEmitter};

use crate::api;
use crate::login;
use crate::prompt::{is_2fa_code, Prompter};

// What the app is asked now, as (step, detail):
//   relay      (previous relay host or "")      answer: [host, code]
//   account    ("")                             answer: [Apple ID, password]
//   deviceCode ("")                             answer: [6 digits]
//   phone      ("id:last two digits,...")       answer: [id]
//   smsCode    ("")                             answer: [6 digits]
//   working    (what is going on)               no answer
//   failed     (why)                            answer: [] to start over
type Step = Arc<Mutex<(String, String)>>;

pub struct SignIn {
    step: Step,
    answers: mpsc::Sender<Vec<String>>,
}

fn expected_answers(step: &str) -> Option<usize> {
    match step {
        "relay" | "account" => Some(2),
        "deviceCode" | "phone" | "smsCode" => Some(1),
        "failed" => Some(0),
        _ => None,
    }
}

#[interface(name = "org.sailbubbles.SignIn1")]
impl SignIn {
    /// The current (step, detail).
    fn state(&self) -> (String, String) {
        self.step.lock().unwrap().clone()
    }

    async fn answer(&self, values: Vec<String>) -> fdo::Result<()> {
        let step = self.step.lock().unwrap().0.clone();
        if expected_answers(&step) != Some(values.len()) {
            return Err(fdo::Error::InvalidArgs("not the answer this step wants".into()));
        }
        if matches!(step.as_str(), "deviceCode" | "smsCode") && !is_2fa_code(&values[0]) {
            return Err(fdo::Error::InvalidArgs("a code has 6 digits".into()));
        }
        self.answers.send(values).await.map_err(|e| fdo::Error::Failed(e.to_string()))
    }

    #[zbus(signal)]
    async fn step_changed(e: &SignalEmitter<'_>, step: &str, detail: &str) -> zbus::Result<()>;
}

struct AppPrompter {
    step: Step,
    answers: mpsc::Receiver<Vec<String>>,
    emitter: SignalEmitter<'static>,
}

impl AppPrompter {
    async fn show(&mut self, step: &str, detail: &str) {
        *self.step.lock().unwrap() = (step.to_string(), detail.to_string());
        let _ = SignIn::step_changed(&self.emitter, step, detail).await;
    }

    async fn ask(&mut self, step: &str, detail: &str) -> Result<Vec<String>> {
        // an answer sent for an earlier step is not one for this one
        while self.answers.try_recv().is_ok() {}
        self.show(step, detail).await;
        self.answers.recv().await.context("sign-in ended")
    }
}

fn take_two(mut v: Vec<String>) -> (String, String) {
    let second = v.pop().unwrap_or_default();
    (v.pop().unwrap_or_default(), second)
}

impl Prompter for AppPrompter {
    async fn relay(&mut self, previous: Option<&str>) -> Result<(String, String)> {
        Ok(take_two(self.ask("relay", previous.unwrap_or("")).await?))
    }

    async fn account(&mut self) -> Result<(String, String)> {
        Ok(take_two(self.ask("account", "").await?))
    }

    async fn device_code(&mut self) -> Result<String> {
        Ok(self.ask("deviceCode", "").await?.remove(0))
    }

    async fn phone(&mut self, phones: &[(u32, String)]) -> Result<u32> {
        let listed: Vec<String> = phones.iter().map(|(id, last)| format!("{id}:{last}")).collect();
        let id: u32 = self.ask("phone", &listed.join(",")).await?.remove(0).parse().context("not a number")?;
        if !phones.iter().any(|(p, _)| *p == id) {
            bail!("no such number");
        }
        Ok(id)
    }

    async fn sms_code(&mut self) -> Result<String> {
        Ok(self.ask("smsCode", "").await?.remove(0))
    }

    async fn working(&mut self, what: &str) {
        self.show("working", what).await;
    }
}

/// Offers sign-in to the app on the daemon's bus name until it succeeded,
/// then gives the name back for the daemon proper.
pub async fn serve(dir: &Path) -> Result<()> {
    let step: Step = Arc::new(Mutex::new(("working".into(), String::new())));
    let (tx, rx) = mpsc::channel(4);
    let bus = zbus::connection::Builder::session()?
        .name(api::NAME)?
        .serve_at(api::PATH, SignIn { step: step.clone(), answers: tx })?
        .build().await
        .context("another sailbubbles-daemon is running, or there is no session bus")?;
    log::info!("waiting for sign-in in the app");
    let mut prompter = AppPrompter { step, answers: rx, emitter: SignalEmitter::new(&bus, api::PATH)?.into_owned() };
    loop {
        match login::login(dir, &mut prompter).await {
            Ok(()) => break,
            Err(e) => {
                // the reason can name the account, so it goes to the app only
                log::error!("signing in failed");
                prompter.ask("failed", &e.to_string()).await?;
            }
        }
    }
    drop(prompter);
    bus.release_name(api::NAME).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixStream;
    use zbus::connection::Builder;

    #[test]
    fn steps_take_their_number_of_answers() {
        assert_eq!(expected_answers("relay"), Some(2));
        assert_eq!(expected_answers("smsCode"), Some(1));
        assert_eq!(expected_answers("failed"), Some(0));
        assert_eq!(expected_answers("working"), None);
    }

    #[tokio::test]
    async fn the_app_answers_the_prompts() {
        let step: Step = Arc::new(Mutex::new(("working".into(), String::new())));
        let (tx, rx) = mpsc::channel(4);
        let (a, b) = UnixStream::pair().unwrap();
        let server = Builder::unix_stream(a).server(zbus::Guid::generate()).unwrap().p2p()
            .serve_at(api::PATH, SignIn { step: step.clone(), answers: tx }).unwrap().build();
        let client = Builder::unix_stream(b).p2p().build();
        let (server, client) = tokio::try_join!(server, client).unwrap();
        let mut prompter = AppPrompter { step, answers: rx, emitter: SignalEmitter::new(&server, api::PATH).unwrap().into_owned() };

        let call = |method: &'static str, values: Vec<&'static str>| {
            let client = client.clone();
            async move {
                client.call_method(None::<&str>, api::PATH, Some("org.sailbubbles.SignIn1"), method, &(values,)).await
            }
        };
        let state = |client: zbus::Connection| async move {
            let reply = client.call_method(None::<&str>, api::PATH, Some("org.sailbubbles.SignIn1"), "State", &()).await.unwrap();
            reply.body().deserialize::<(String, String)>().unwrap()
        };

        let asked = tokio::spawn(async move {
            let relay = prompter.relay(Some("https://relay.example")).await.unwrap();
            let phone = prompter.phone(&[(3, "42".into()), (7, "17".into())]).await.unwrap();
            let code = prompter.sms_code().await.unwrap();
            (relay, phone, code)
        });
        while state(client.clone()).await.0 != "relay" {
            tokio::task::yield_now().await;
        }
        assert_eq!(state(client.clone()).await.1, "https://relay.example");
        assert!(call("Answer", vec!["only one"]).await.is_err());
        call("Answer", vec!["", "secret"]).await.unwrap();
        while state(client.clone()).await.0 != "phone" {
            tokio::task::yield_now().await;
        }
        assert_eq!(state(client.clone()).await.1, "3:42,7:17");
        call("Answer", vec!["7"]).await.unwrap();
        while state(client.clone()).await.0 != "smsCode" {
            tokio::task::yield_now().await;
        }
        assert!(call("Answer", vec!["12a456"]).await.is_err());
        call("Answer", vec!["123456"]).await.unwrap();
        let (relay, phone, code) = asked.await.unwrap();
        assert_eq!((relay, phone, code.as_str()), ((String::new(), "secret".to_string()), 7, "123456"));
    }
}
