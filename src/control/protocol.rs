use crate::config::ActiveConfig;

pub const PROTOCOL_VERSION: u32 = 1;
pub(super) const MAX_CONFIG_SOURCE_SIZE: usize = 4096;
pub(super) const SWITCH_CONFIG_PREFIX: &[u8] = b"switch-config\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlState {
    pub uhid_ready: bool,
    pub needs_config: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlRequest {
    SwitchConfig(ActiveConfig),
}

impl ControlRequest {
    pub(super) fn ok_packet(&self) -> &'static [u8] {
        match self {
            Self::SwitchConfig(_) => b"ok switch-config",
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::SwitchConfig(config) => {
                let mut packet = Vec::with_capacity(
                    SWITCH_CONFIG_PREFIX.len() + config.source().len() + 1 + config.content().len(),
                );
                packet.extend_from_slice(SWITCH_CONFIG_PREFIX);
                packet.extend_from_slice(config.source().as_bytes());
                packet.push(0);
                packet.extend_from_slice(config.content().as_bytes());
                packet
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerPacket {
    Hello(ControlState),
    State(ControlState),
    OkSwitchConfig,
    Error { code: String, message: String },
}

fn bool_digit(value: bool) -> char {
    if value {
        '1'
    } else {
        '0'
    }
}

pub(super) fn hello_packet(state: ControlState) -> Vec<u8> {
    format!(
        "hello version={PROTOCOL_VERSION} uhid_ready={} needs_config={}",
        bool_digit(state.uhid_ready),
        bool_digit(state.needs_config)
    )
    .into_bytes()
}

pub(super) fn state_packet(state: ControlState) -> Vec<u8> {
    format!(
        "state uhid_ready={} needs_config={}",
        bool_digit(state.uhid_ready),
        bool_digit(state.needs_config)
    )
    .into_bytes()
}

fn parse_state_fields(input: &str) -> Result<ControlState, String> {
    let mut fields = input.split_ascii_whitespace();
    let ready = fields
        .next()
        .and_then(|field| field.strip_prefix("uhid_ready="))
        .ok_or_else(|| "missing uhid_ready field".to_string())?;
    let needs = fields
        .next()
        .and_then(|field| field.strip_prefix("needs_config="))
        .ok_or_else(|| "missing needs_config field".to_string())?;
    if fields.next().is_some() {
        return Err("unexpected state fields".to_string());
    }
    let parse_bool = |value: &str| match value {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(format!("invalid boolean value {value:?}")),
    };
    Ok(ControlState {
        uhid_ready: parse_bool(ready)?,
        needs_config: parse_bool(needs)?,
    })
}

pub fn parse_server_packet(packet: &[u8]) -> Result<ServerPacket, String> {
    let text = std::str::from_utf8(packet).map_err(|_| "server packet is not UTF-8".to_string())?;
    if let Some(fields) = text.strip_prefix("hello ") {
        let fields = fields
            .strip_prefix(&format!("version={PROTOCOL_VERSION} "))
            .ok_or_else(|| "unsupported control protocol version".to_string())?;
        return parse_state_fields(fields).map(ServerPacket::Hello);
    }
    if let Some(fields) = text.strip_prefix("state ") {
        return parse_state_fields(fields).map(ServerPacket::State);
    }
    if text == "ok switch-config" {
        return Ok(ServerPacket::OkSwitchConfig);
    }
    if let Some(error) = text.strip_prefix("error ") {
        let (code, message) = error
            .split_once(' ')
            .ok_or_else(|| "malformed error packet".to_string())?;
        if code.is_empty() || message.is_empty() {
            return Err("malformed error packet".to_string());
        }
        return Ok(ServerPacket::Error {
            code: code.to_string(),
            message: message.to_string(),
        });
    }
    Err(format!("unknown server packet: {text:?}"))
}

pub(super) fn parse_request(packet: &[u8]) -> Result<ControlRequest, String> {
    if let Some(payload) = packet.strip_prefix(SWITCH_CONFIG_PREFIX) {
        let separator = payload
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| "switch-config source delimiter is missing".to_string())?;
        let (source, content_with_separator) = payload.split_at(separator);
        if source.is_empty() {
            return Err("switch-config source is empty".to_string());
        }
        if source.len() > MAX_CONFIG_SOURCE_SIZE {
            return Err("switch-config source exceeds size limit".to_string());
        }
        let source = std::str::from_utf8(source)
            .map_err(|_| "switch-config source is not UTF-8".to_string())?;
        let content = std::str::from_utf8(&content_with_separator[1..])
            .map_err(|_| "switch-config content is not UTF-8".to_string())?;
        let active_config = ActiveConfig::from_content(source.to_string(), content.to_string())?;
        return Ok(ControlRequest::SwitchConfig(active_config));
    }
    let command = std::str::from_utf8(packet).map_err(|_| "request is not UTF-8".to_string())?;
    Err(format!("unknown command: {command:?}"))
}
