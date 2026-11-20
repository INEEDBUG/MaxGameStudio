//! Authentication contracts for the desktop loopback service.

use std::borrow::Cow;
use subtle::ConstantTimeEq;

pub const SESSION_TOKEN_HEADER: &str = "X-CS2-Insight-Token";
const SESSION_QUERY_KEY: &str = "_session";
const WEBSOCKET_PROTOCOL: &str = "cs2-insight";

pub struct SessionAuth {
    token: Option<Box<str>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebSocketAuthorization {
    Reject,
    Accept,
    AcceptWithProtocol(&'static str),
}

impl SessionAuth {
    pub fn new(token: &str) -> Self {
        let token = token.trim();
        Self {
            token: (!token.is_empty()).then(|| Box::from(token)),
        }
    }

    fn matches(&self, supplied: &str) -> bool {
        let Some(token) = &self.token else {
            return true;
        };
        let supplied = supplied.trim().as_bytes();
        bool::from(supplied.ct_eq(token.as_bytes()))
    }

    pub fn authorize_http(&self, header: Option<&str>, query: &[u8]) -> bool {
        if self.token.is_none() {
            return true;
        }
        if let Some(header) = header.filter(|value| !value.is_empty()) {
            return self.matches(header);
        }
        form_urlencoded::parse(query)
            .filter(|(name, _)| name == SESSION_QUERY_KEY)
            .last()
            .is_some_and(|(_, value)| self.matches(&value))
    }

    pub fn authorize_websocket(&self, offered: Option<&str>) -> WebSocketAuthorization {
        if self.token.is_none() {
            return WebSocketAuthorization::Accept;
        }
        let Some(offered) = offered else {
            return WebSocketAuthorization::Reject;
        };
        let protocols = offered.split(',').map(str::trim);
        if protocols.clone().any(|value| value == WEBSOCKET_PROTOCOL)
            && protocols
                .filter(|value| !value.is_empty())
                .any(|value| self.matches(value))
        {
            WebSocketAuthorization::AcceptWithProtocol(WEBSOCKET_PROTOCOL)
        } else {
            WebSocketAuthorization::Reject
        }
    }

    pub fn overlay_fragment(&self) -> String {
        self.token
            .as_deref()
            .map(|token| format!("#session={token}"))
            .unwrap_or_default()
    }
}

pub fn strip_session_query(query: &[u8]) -> Cow<'_, [u8]> {
    if !form_urlencoded::parse(query).any(|(name, _)| name == SESSION_QUERY_KEY) {
        return Cow::Borrowed(query);
    }
    let mut retained = form_urlencoded::Serializer::new(String::new());
    retained
        .extend_pairs(form_urlencoded::parse(query).filter(|(name, _)| name != SESSION_QUERY_KEY));
    Cow::Owned(retained.finish().into_bytes())
}
