use std::collections::HashMap;

use omnisette::{AnisetteError, AnisetteProvider};
use serde::Deserialize;

// Anisette minted by the relay's Mac through AOSKit. Public anisette servers
// run Android CoreADI, which Apple rejects (-80045) next to a Mac identity.
pub struct RelayAnisette {
    host: String,
    code: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Reply {
    Headers { headers: HashMap<String, String> },
    Error { error: String },
}

impl RelayAnisette {
    pub fn new(host: String, code: String) -> Self {
        RelayAnisette { host, code }
    }
}

fn headers_from(reply: Reply) -> Result<HashMap<String, String>, AnisetteError> {
    match reply {
        Reply::Headers { headers } => Ok(headers),
        Reply::Error { error } => Err(anyhow::anyhow!("the Mac could not mint anisette: {error}").into()),
    }
}

impl AnisetteProvider for RelayAnisette {
    async fn get_anisette_headers(&mut self) -> Result<HashMap<String, String>, AnisetteError> {
        let reply = reqwest::Client::new()
            .post(format!("{}/api/v1/bridge/get-anisette-data", self.host))
            .bearer_auth(&self.code)
            .header("Content-Length", "0")
            .send().await?
            .error_for_status()?
            .json::<Reply>().await?;
        headers_from(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn serve_once(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = [0u8; 4096];
            let n = s.read(&mut req).unwrap();
            let req = String::from_utf8_lossy(&req[..n]);
            assert!(req.starts_with("POST /api/v1/bridge/get-anisette-data"));
            assert!(req.contains("Bearer CODE"));
            write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                   body.len()).unwrap();
        });
        format!("http://{addr}")
    }

    async fn fetch(body: &'static str) -> Result<HashMap<String, String>, AnisetteError> {
        // the VM's proxy cannot reach our loopback listener
        std::env::set_var("NO_PROXY", "127.0.0.1");
        RelayAnisette::new(serve_once(body), "CODE".into()).get_anisette_headers().await
    }

    #[tokio::test]
    async fn headers_come_back() {
        let h = fetch(r#"{"headers":{"X-Apple-I-MD":"otp","X-Apple-I-MD-M":"mid"}}"#).await.unwrap();
        assert_eq!(h["X-Apple-I-MD"], "otp");
        assert_eq!(h["X-Apple-I-MD-M"], "mid");
    }

    #[tokio::test]
    async fn old_provider_is_reported() {
        let e = fetch(r#"{"error":"unknown command \"get-anisette-data\""}"#).await.unwrap_err();
        assert!(e.to_string().contains("could not mint anisette"), "{e}");
    }
}
