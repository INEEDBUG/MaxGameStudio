use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures_util::{FutureExt, SinkExt, StreamExt};
use hyper::{
    header::{HeaderValue, SEC_WEBSOCKET_PROTOCOL},
    Uri,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};
use tokio::net::TcpStream;
use tokio_websockets::{ClientBuilder, Limits, Message, WebSocketStream};

pub(crate) const OUTPUTS_SUBSCRIPTION: u64 = 1 << 6;

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
        Self::identify(uri, stream, settings.password, 0).await
    }

    pub(crate) async fn connect_recording(settings: ObsSettings) -> io::Result<Self> {
        let uri = endpoint(&settings)?;
        let host = settings.host.trim();
        let port = settings.port;
        // The recording directory returned by OBS must belong to this
        // machine. Resolve the reserved localhost name without DNS.
        let stream = if host.eq_ignore_ascii_case("localhost") {
            let loopback = [
                SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
                SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
            ];
            TcpStream::connect(&loopback[..]).await
        } else {
            let ip = host
                .parse::<IpAddr>()
                .ok()
                .or_else(|| {
                    let inside = host.strip_prefix('[')?.strip_suffix(']')?;
                    inside.parse::<Ipv6Addr>().ok().map(IpAddr::V6)
                })
                .filter(IpAddr::is_loopback)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "OBS recording requires a loopback host",
                    )
                })?;
            TcpStream::connect(SocketAddr::new(ip, port)).await
        }
        .map_err(|_| io::Error::other("Unable to connect to local OBS"))?;
        Self::identify(uri, stream, settings.password, OUTPUTS_SUBSCRIPTION).await
    }

    async fn identify(
        uri: Uri,
        stream: TcpStream,
        password: String,
        subscriptions: u64,
    ) -> io::Result<Self> {
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
        let mut identify = json!({"rpcVersion":1,"eventSubscriptions":subscriptions});
        if let Some(authentication) = hello["d"].get("authentication").filter(|x| !x.is_null()) {
            let salt = authentication["salt"]
                .as_str()
                .filter(|x| !x.is_empty())
                .ok_or_else(bad_response)?;
            let challenge = authentication["challenge"]
                .as_str()
                .filter(|x| !x.is_empty())
                .ok_or_else(bad_response)?;
            if password.is_empty() {
                return Err(io::Error::other(
                    "OBS authentication requires the configured password",
                ));
            }
            identify["authentication"] = json!(authenticate(&password, salt, challenge));
        }
        client.send(json!({"op":1,"d":identify})).await?;
        let reply = client.receive().await?;
        if reply["op"] != 2 || reply["d"]["negotiatedRpcVersion"] != 1 {
            return Err(bad_response());
        }
        Ok(client)
    }

    pub(crate) async fn request(&mut self, method: &str, data: Value) -> io::Result<Value> {
        self.request_observing(method, data, |_| Ok(())).await
    }

    pub(crate) async fn request_observing(
        &mut self,
        method: &str,
        data: Value,
        mut on_event: impl FnMut(&Value) -> io::Result<()>,
    ) -> io::Result<Value> {
        self.sequence += 1;
        let id = self.sequence.to_string();
        self.send(json!({"op":6,"d":{"requestType":method,"requestId":id,"requestData":data}}))
            .await?;
        loop {
            let mut incoming = self.receive().await?;
            if incoming["op"] == 5 {
                on_event(&incoming["d"])?;
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

    /// Process events already available on the socket after a reply. This
    /// narrows the interval before a follow-up mutation, but cannot make it atomic.
    pub(crate) fn drain_events(
        &mut self,
        mut on_event: impl FnMut(&Value) -> io::Result<()>,
    ) -> io::Result<()> {
        for _ in 0..128 {
            let Some(received) = self.receive().now_or_never() else {
                return Ok(());
            };
            let event = received?;
            if event["op"] != 5 {
                return Err(bad_response());
            }
            on_event(&event["d"])?;
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Too many queued OBS events",
        ))
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
