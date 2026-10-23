use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use omnisette::AnisetteClient;
use reqwest::Url;
use rustpush::{
    authenticate_apple, login_apple_delegates, register, APSConnection,
    APSConnectionResource, AppleAccount, CircleClientSession, DebugMutex,
    IDSNGMIdentity, IdmsAuthListener, IdmsMessage, LoginDelegate, LoginState, OSConfig,
    PushError, RelayConfig, VerifyBody, MADRID_SERVICE,
};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::Mutex;

use crate::anisette::RelayAnisette;
use crate::prompt::Prompter;
use crate::random;
use crate::state::{self, Saved};

type Account = Arc<DebugMutex<AppleAccount<RelayAnisette>>>;

const CIRCLE_TIMEOUT: Duration = Duration::from_secs(60);
// Identity blobs are sealed with a keystore key named after this tag; the
// name predates SailBubbles and stays, or saved logins could not be opened.
pub const IDENTITY_TAG: &str = "openbubbles";

fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn uuid_v4(mut b: [u8; 16]) -> String {
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex_upper(&b);
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

pub fn new_uuid() -> std::io::Result<String> {
    Ok(uuid_v4(random::bytes()?))
}

// Values as OpenBubbles sends them for a relay-backed Mac.
// The relay code travels in plain HTTP, so the relay must be on the phone's own
// network: a home network, a VPN such as Tailscale, or the phone itself.
fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private() || v4.is_loopback() || v4.is_link_local() || (a == 100 && (64..128).contains(&b))
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback() || first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80
        }
    }
}

async fn check_local(address: &str) -> Result<()> {
    let url = Url::parse(address).context("not an address like http://192.168.1.23:8000")?;
    let port = url.port_or_known_default().unwrap_or(80);
    let host = url.host_str().context("no host in the relay address")?;
    let ips: Vec<IpAddr> = match host.trim_start_matches('[').trim_end_matches(']').parse() {
        Ok(ip) => vec![ip],
        Err(_) => tokio::net::lookup_host((host, port)).await
            .context("the relay's name cannot be found")?.map(|a| a.ip()).collect(),
    };
    if ips.is_empty() || !ips.into_iter().all(is_local) {
        bail!("the relay must be on your own network, like your Mac at home");
    }
    Ok(())
}

async fn relay_config(host: String, code: String) -> Result<RelayConfig> {
    check_local(&host).await?;
    let version = match RelayConfig::get_versions(&host, &code, &None).await {
        Ok(v) => v,
        Err(PushError::DeviceNotFound) => bail!("no Mac is connected to this relay with that code"),
        Err(PushError::RelayError(401, _)) => bail!("the relay asks for an account token; use one of your own"),
        Err(e) => return Err(e).context("relay request failed"),
    };
    Ok(RelayConfig {
        version,
        icloud_ua: "com.apple.iCloudHelper/282 CFNetwork/1408.0.4 Darwin/22.5.0".into(),
        aoskit_version: "com.apple.AOSKit/282 (com.apple.accountsd/113)".into(),
        dev_uuid: uuid_v4(random::bytes()?),
        protocol_version: 1660,
        host,
        code,
        beeper_token: None,
        udid: Some(hex_upper(&random::bytes::<32>()?)),
    })
}

async fn device_2fa(account: &Account, conn: &APSConnection, prompter: &mut impl Prompter) -> Result<LoginState> {
    let dsid = account.lock().await.persisted.as_ref().context("no account data after login")?.dsid;
    let idms = IdmsAuthListener::new(conn.clone()).await;
    let mut msgs = conn.messages_cont.subscribe();
    let mut session = CircleClientSession::new(dsid, account.clone(), conn.get_token().await).await?;

    session.send_code(&prompter.device_code().await?).await?;
    prompter.working("Waiting for Apple to confirm the code").await;

    let circle = async {
        loop {
            let msg = match msgs.recv().await {
                Ok(msg) => msg,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => bail!("push connection closed"),
            };
            if let Some(IdmsMessage::CircleRequest(req, _)) = idms.handle(msg)? {
                if let Some(state) = session.handle_circle_request(&req).await? {
                    return Ok(state);
                }
            }
        }
    };
    tokio::time::timeout(CIRCLE_TIMEOUT, circle).await
        .map_err(|_| anyhow!("timed out waiting for Apple to confirm the code"))?
}

async fn request_sms(account: &Account, prompter: &mut impl Prompter) -> Result<LoginState> {
    let account = account.lock().await;
    let phones: Vec<(u32, String)> = account.get_auth_extras().await?.trusted_phone_numbers.into_iter()
        .map(|p| (p.id, p.last_two_digits)).collect();
    let phone = match phones.as_slice() {
        [] => bail!("Apple offers no trusted phone number for SMS"),
        [(only, _)] => *only,
        _ => prompter.phone(&phones).await?,
    };
    Ok(account.send_sms_2fa_to_devices(phone).await?)
}

async fn verify_sms(account: &Account, body: VerifyBody, prompter: &mut impl Prompter) -> Result<LoginState> {
    let code = prompter.sms_code().await?;
    prompter.working("Checking the code").await;
    Ok(account.lock().await.verify_sms_2fa(code, body).await?)
}

async fn sign_in(relay: &RelayConfig, conn: &APSConnection, prompter: &mut impl Prompter) -> Result<Account> {
    let push = conn.state.read().await.clone();
    let anisette = Arc::new(Mutex::new(AnisetteClient::new(
        RelayAnisette::new(relay.host.clone(), relay.code.clone()))));
    let account = AppleAccount::new_with_anisette(
        relay.get_gsa_config(&push, false), anisette, None, Box::new(|_| {}))?;
    let account = Arc::new(DebugMutex::new(account));

    let (apple_id, password) = prompter.account().await?;
    let hashed = Sha256::digest(password.as_bytes()).to_vec();
    prompter.working("Signing in to Apple").await;
    let mut state = account.lock().await.login_email_pass(&apple_id, &hashed).await?;
    loop {
        // Apple sometimes asks for an extra step after 2FA even though it has
        // already issued the token we need; OpenBubbles treats that as signed in.
        if account.lock().await.get_pet().is_some() {
            return Ok(account);
        }
        state = match state {
            LoginState::NeedsDevice2FA | LoginState::Needs2FAVerification => device_2fa(&account, conn, prompter).await?,
            LoginState::NeedsSMS2FA => request_sms(&account, prompter).await?,
            LoginState::NeedsSMS2FAVerification(body) => verify_sms(&account, body, prompter).await?,
            LoginState::NeedsLogin => bail!("Apple did not accept the Apple ID or password"),
            LoginState::NeedsExtraStep(step) => bail!("Apple wants a step this client cannot do: {step}"),
            LoginState::LoggedIn => bail!("signed in but Apple issued no token"),
        };
    }
}

/// Signs in to Apple and registers for iMessage, asking `prompter` for what
/// it needs; saves the result in `dir`.
pub async fn login(dir: &Path, prompter: &mut impl Prompter) -> Result<()> {
    let previous = state::load(dir).ok().flatten().map(|s| (s.relay.host, s.relay.code));
    let (host, code) = prompter.relay(previous.as_ref().map(|(h, _)| h.as_str())).await?;
    let host = host.trim().to_string();
    if host.is_empty() {
        bail!("enter the address of the relay on your Mac");
    }
    // signing in again keeps the relay code unless a new one is given
    let code = match previous {
        Some((h, c)) if code.is_empty() && h == host => c,
        _ => code,
    };
    prompter.working("Connecting to the relay").await;
    let relay = Arc::new(relay_config(host, code).await?);
    log::info!("relay ok");

    let (conn, err) = APSConnectionResource::new(relay.clone(), None).await;
    if let Some(e) = err {
        return Err(e).context("push connection failed");
    }
    log::info!("push connected");

    let account = sign_in(&relay, &conn, prompter).await?;
    log::info!("apple id signed in");
    prompter.working("Registering for iMessage").await;

    let mut account = account.lock().await;
    account.update_postdata("Apple Device", None, &["icloud", "imessage", "facetime"]).await?;
    let delegates = login_apple_delegates(&*account, None, &*relay, &[LoginDelegate::IDS]).await?;
    let user = authenticate_apple(delegates.ids.context("Apple returned no IDS login")?, &*relay).await?;

    let identity = IDSNGMIdentity::new()?;
    let mut users = vec![user];
    let push = conn.state.read().await.clone();
    match register(&*relay, &push, &[&MADRID_SERVICE], &mut users, &identity).await {
        Err(PushError::CustomerMessage(alert)) => bail!("Apple refused registration: {alert:?}"),
        r => r.context("iMessage registration failed")?,
    }
    log::info!("imessage registered");

    state::save(dir, &Saved {
        relay: (*relay).clone(),
        push,
        users,
        identity: identity.save(IDENTITY_TAG)?,
    })?;
    log::info!("login saved");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_has_v4_layout() {
        let u = uuid_v4([0xff; 16]);
        assert_eq!(u, "FFFFFFFF-FFFF-4FFF-BFFF-FFFFFFFFFFFF");
        let u = uuid_v4([0; 16]);
        assert_eq!(u, "00000000-0000-4000-8000-000000000000");
    }

    #[test]
    fn only_local_addresses_count() {
        for ip in ["192.168.1.23", "10.0.0.2", "172.16.5.1", "127.0.0.1", "169.254.1.1", "100.101.102.103",
                   "::1", "fd7a:115c:a1e0::1", "fe80::1"] {
            assert!(is_local(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "100.128.0.1", "172.32.0.1", "2001:db8::1", "2a00:1450::1"] {
            assert!(!is_local(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn the_relay_address_must_be_local() {
        assert!(check_local("http://192.168.1.23:8000").await.is_ok());
        assert!(check_local("http://localhost:8000").await.is_ok());
        assert!(check_local("http://[fd00::5]:8000").await.is_ok());
        assert!(check_local("https://8.8.8.8").await.is_err());
        assert!(check_local("192.168.1.23:8000").await.is_err());
        assert!(check_local("not an address").await.is_err());
    }

    #[test]
    fn hex_is_upper() {
        assert_eq!(hex_upper(&[0x0a, 0xbc]), "0ABC");
    }
}
