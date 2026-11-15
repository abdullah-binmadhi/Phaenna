//! Native AirPlay 2 & DLNA/UPnP Direct TV Casting Engine.

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CastProtocol {
    AirPlay,
    Dlna,
    Custom,
}

impl CastProtocol {
    pub fn badge(&self) -> &'static str {
        match self {
            CastProtocol::AirPlay => "AirPlay 2",
            CastProtocol::Dlna => "DLNA / UPnP",
            CastProtocol::Custom => "Network TV",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastDevice {
    pub id: String,
    pub name: String,
    pub protocol: CastProtocol,
    pub host: String,
    pub port: u16,
    pub control_path: String,
    pub is_active: bool,
}

#[allow(dead_code)]
impl CastDevice {
    pub fn new_airplay(name: &str, host: &str, port: u16) -> Self {
        Self {
            id: format!("airplay-{}-{}", host, port),
            name: name.to_string(),
            protocol: CastProtocol::AirPlay,
            host: host.to_string(),
            port,
            control_path: "/play".to_string(),
            is_active: false,
        }
    }

    pub fn new_dlna(name: &str, host: &str, port: u16, control_url: &str) -> Self {
        Self {
            id: format!("dlna-{}-{}", host, port),
            name: name.to_string(),
            protocol: CastProtocol::Dlna,
            host: host.to_string(),
            port,
            control_path: control_url.to_string(),
            is_active: false,
        }
    }

    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Resolve direct best muxed HTTP video stream URL via `yt-dlp -g` so smart TVs can play directly.
pub async fn resolve_tv_stream_url(video_id: &str) -> Result<String, String> {
    let bin = crate::api::player::find_binary("yt-dlp")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "yt-dlp".to_string());
    let url = crate::api::player::youtube_watch_url(video_id);
    let out = tokio::process::Command::new(bin)
        .args(["-g", "-f", "best[ext=mp4][vcodec!=none][acodec!=none]/best", &url])
        .output()
        .await
        .map_err(|e| format!("yt-dlp spawn error: {}", e))?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with("http"))
        .ok_or_else(|| "Unable to resolve single-stream URL for TV".to_string())
}

/// Discover UPnP/DLNA smart TVs on the LAN via SSDP M-SEARCH UDP multicast.
pub async fn discover_devices(timeout_ms: u64) -> Vec<CastDevice> {
    let mut found = Vec::new();
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0").await else {
        return found;
    };
    let _ = sock.set_broadcast(true);
    let target: SocketAddr = "239.255.255.250:1900".parse().unwrap();
    let payload = b"M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: urn:schemas-upnp-org:device:MediaRenderer:1\r\n\r\n";
    let _ = sock.send_to(payload, target).await;

    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    let mut buf = [0u8; 2048];
    let mut seen = std::collections::HashSet::new();

    while tokio::time::Instant::now() < deadline {
        let rem = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(rem, sock.recv_from(&mut buf)).await {
            Ok(Ok((n, addr))) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                if let Some(dev) = parse_ssdp_response(&text, &addr.ip().to_string()) {
                    if seen.insert(dev.id.clone()) {
                        found.push(dev);
                    }
                }
            }
            _ => break,
        }
    }
    found
}

pub fn parse_ssdp_response(resp: &str, peer_ip: &str) -> Option<CastDevice> {
    let mut location = None;
    let mut server_name = None;
    for line in resp.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("location:") {
            location = Some(line[9..].trim().to_string());
        } else if lower.starts_with("server:") {
            server_name = Some(line[7..].trim().to_string());
        }
    }
    let loc = location?;
    let port = loc
        .split(':')
        .nth(2)
        .and_then(|s| s.split('/').next())
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(1400);
    let path = loc
        .find("://")
        .and_then(|i| loc[i + 3..].find('/').map(|j| &loc[i + 3 + j..]))
        .unwrap_or("/AVTransport/control");

    let name = server_name
        .map(|s| format!("Smart TV ({})", s.chars().take(28).collect::<String>()))
        .unwrap_or_else(|| format!("DLNA Receiver ({})", peer_ip));

    Some(CastDevice::new_dlna(&name, peer_ip, port, path))
}

pub async fn start_casting(
    device: &CastDevice,
    stream_url: &str,
    start_pos: f64,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    match device.protocol {
        CastProtocol::AirPlay => {
            let endpoint = format!("http://{}:{}/play", device.host, device.port);
            let body = format!(
                "Content-Location: {}\nStart-Position: {:.3}\n",
                stream_url,
                start_pos.max(0.0)
            );
            let resp = client
                .post(&endpoint)
                .header("Content-Type", "text/parameters")
                .body(body)
                .send()
                .await
                .map_err(|e| format!("AirPlay error: {}", e))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(format!("AirPlay HTTP {}", resp.status()))
            }
        }
        CastProtocol::Dlna | CastProtocol::Custom => {
            let endpoint = format!("http://{}:{}{}", device.host, device.port, device.control_path);
            let soap = format!(
                r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <s:Body>
    <u:SetAVTransportURI xmlns:u="urn:schemas-upnp-org:service:AVTransport:1">
      <InstanceID>0</InstanceID>
      <CurrentURI>{}</CurrentURI>
      <CurrentURIMetaData></CurrentURIMetaData>
    </u:SetAVTransportURI>
  </s:Body>
</s:Envelope>"#,
                stream_url.replace('&', "&amp;")
            );
            let resp = client
                .post(&endpoint)
                .header("Content-Type", "text/xml; charset=\"utf-8\"")
                .header(
                    "SOAPACTION",
                    "\"urn:schemas-upnp-org:service:AVTransport:1#SetAVTransportURI\"",
                )
                .body(soap)
                .send()
                .await
                .map_err(|e| format!("DLNA error: {}", e))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(format!("DLNA HTTP {}", resp.status()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ssdp_response() {
        let raw = "HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.42:8200/MediaRenderer/desc.xml\r\nSERVER: LG WebOS TV UPnP/1.0\r\n\r\n";
        let dev = parse_ssdp_response(raw, "192.168.1.42").unwrap();
        assert_eq!(dev.host, "192.168.1.42");
        assert_eq!(dev.port, 8200);
        assert!(dev.name.contains("LG WebOS TV"));
    }
}
