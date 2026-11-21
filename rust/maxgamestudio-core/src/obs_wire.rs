use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures_util::{SinkExt, StreamExt};
use hyper::{
    header::{HeaderValue, SEC_WEBSOCKET_PROTOCOL},
    Uri,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{io, net::Ipv6Addr};
use tokio::net::TcpStream;
use tokio_websockets::{ClientBuilder, Limits, Message, WebSocketStream};

#[derive(Clone, Deserialize, Eq, PartialEq)]
pub(crate) struct ObsSettings {
    host: String,
    port: u16,
    password: String,
}

impl ObsSettings {
    pub(crate) fn with_check_input(&self, host: &str, port: u16, password: &str) -> Self {
        let host = host.trim();
        let password = password.trim();
        Self {
            host: if host.is_empty() { &self.host } else { host }.to_owned(),
            port,
            password: if password.is_empty() || password.starts_with("****") {
                &self.password
            } else {
                password
            }
            .to_owned(),
        }
    }
}

pub(crate) struct ObsRpc {
    socket: WebSocketStream<TcpStream>,
    sequence: u64,
}

impl ObsRpc {
    pub(crate) async fn connect(settings: ObsSettings) -> io::Result<Self> {
        let uri = endpoint(&settings)?;
        let host = uri
            .host()
            .ok_or_else(|| io::Error::other("Invalid OBS host"))?;
        let stream = TcpStream::connect((host.trim_matches(['[', ']']), settings.port))
            .await
            .map_err(|_| io::Error::other("Unable to connect to OBS"))?;
        let (socket, _) = ClientBuilder::from_uri(uri)
            .add_header(
                SEC_WEBSOCKET_PROTOCOL,
                HeaderValue::from_static("obswebsocket.json"),
            )
            .map_err(|_| io::Error::other("Unable to configure the OBS connection"))?
            .limits(Limits::default().max_payload_len(Some(1024 * 1024)))
            .connect_on(stream)
            .await
            .map_err(|_| io::Error::other("Unable to connect to OBS"))?;
        let mut client = Self {
            socket,
            sequence: 0,
        };
        let hello = client.receive().await?;
        if hello["op"] != 0
            || !hello["d"]["rpcVersion"]
                .as_u64()
                .is_some_and(|version| version >= 1)
        {
            return Err(bad_response());
        }
        let mut identify = json!({"rpcVersion":1,"eventSubscriptions":0});
        if let Some(authentication) = hello["d"].get("authentication").filter(|x| !x.is_null()) {
            let salt = authentication["salt"]
                .as_str()
                .filter(|x| !x.is_empty())
                .ok_or_else(bad_response)?;
            let challenge = authentication["challenge"]
                .as_str()
                .filter(|x| !x.is_empty())
                .ok_or_else(bad_response)?;
            if settings.password.is_empty() {
                return Err(io::Error::other(
                    "OBS authentication requires the configured password",
                ));
            }
            identify["authentication"] = json!(authenticate(&settings.password, salt, challenge));
        }
        client.send(json!({"op":1,"d":identify})).await?;
        let reply = client.receive().await?;
        if reply["op"] != 2 || reply["d"]["negotiatedRpcVersion"] != 1 {
            return Err(bad_response());
        }
        Ok(client)
    }

    pub(crate) async fn request(&mut self, method: &str, data: Value) -> io::Result<Value> {
        self.sequence += 1;
        let id = self.sequence.to_string();
        self.send(json!({"op":6,"d":{"requestType":method,"requestId":id,"requestData":data}}))
            .await?;
        loop {
            let mut incoming = self.receive().await?;
            if incoming["op"] == 5 {
                continue;
            }
            if incoming["op"] != 7
                || incoming["d"]["requestId"] != id
                || incoming["d"]["requestType"] != method
            {
                return Err(bad_response());
            }
            if incoming["d"]["requestStatus"]["result"] != true {
                return Err(io::Error::other("OBS rejected the request"));
            }
            return Ok(incoming["d"]
                .get_mut("responseData")
                .map(Value::take)
                .unwrap_or(Value::Null));
        }
    }

    async fn send(&mut self, message: Value) -> io::Result<()> {
        self.socket
            .send(Message::text(message.to_string()))
            .await
            .map_err(|_| io::Error::other("Unable to send an OBS request"))
    }

    async fn receive(&mut self) -> io::Result<Value> {
        while let Some(message) = self.socket.next().await {
            let message = message.map_err(|_| bad_response())?;
            if let Some(text) = message.as_text() {
                return serde_json::from_str(text).map_err(|_| bad_response());
            }
            if !message.is_ping() && !message.is_pong() {
                return Err(bad_response());
            }
        }
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "OBS closed its connection",
        ))
    }
}

fn endpoint(settings: &ObsSettings) -> io::Result<Uri> {
    let host = settings.host.trim();
    if host.is_empty() || host.contains(['/', '\\', '#', '?', '@']) || settings.port == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid OBS host or port",
        ));
    }
    let host = if host.parse::<Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    format!("ws://{host}:{}/", settings.port)
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid OBS host or port"))
}

fn authenticate(password: &str, salt: &str, challenge: &str) -> String {
    let secret = STANDARD.encode(
        Sha256::new()
            .chain_update(password)
            .chain_update(salt)
            .finalize(),
    );
    STANDARD.encode(
        Sha256::new()
            .chain_update(secret)
            .chain_update(challenge)
            .finalize(),
    )
}

fn bad_response() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid OBS WebSocket response")
}
