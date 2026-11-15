//! Zero-Dependency LAN Mobile Companion Remote (`phaenna remote`, default port 8485).
//! Serves a self-contained mobile touch UI over local Wi-Fi, exposes bidirectional WebSocket
//! live state sync, renders terminal ASCII QR codes, and drives active `mpv` IPC sockets.

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, RwLock};
use tokio_tungstenite::tungstenite::Message;

#[cfg(unix)]
type IpcStream = tokio::net::UnixStream;
#[cfg(windows)]
type IpcStream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(not(any(unix, windows)))]
type IpcStream = tokio::io::DuplexStream;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RemotePlaybackState {
    pub is_playing: bool,
    pub video_id: String,
    pub title: String,
    pub channel: String,
    pub thumb_url: Option<String>,
    pub time_pos: f64,
    pub duration: f64,
    pub volume: i32,
    pub speed: f64,
    pub sub_active: bool,
    pub sponsorblock_skips: u32,
    pub shader_mode: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum RemoteAction {
    TogglePause,
    Seek { seconds: f64 },
    SetVolume { delta: i32 },
    SetSpeed { speed: f64 },
    ToggleSub,
    SkipForward { seconds: f64 },
}

pub struct RemoteServerHandle {
    pub url: String,
    pub qr_lines: Vec<String>,
    pub state: Arc<RwLock<RemotePlaybackState>>,
    pub active_sock: Arc<RwLock<Option<String>>>,
    pub state_tx: broadcast::Sender<String>,
}

/// Discover non-loopback LAN IPv4 address using a zero-packet UDP socket connect trick.
pub fn get_local_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}

/// Generate compact terminal half-block ASCII/Unicode QR code lines for a URL.
pub fn generate_terminal_qr(url: &str) -> Vec<String> {
    let Ok(code) = qrcode::QrCode::new(url.as_bytes()) else {
        return vec![url.to_string()];
    };
    let width = code.width();
    let mut lines = Vec::new();

    // Pad with 1 quiet module border and pack 2 rows per character via half-blocks
    let total_w = width + 2;
    let total_h = width + 2;
    let is_dark = |x: isize, y: isize| -> bool {
        if x < 0 || y < 0 || x >= width as isize || y >= width as isize {
            false
        } else {
            code[(x as usize, y as usize)] == qrcode::Color::Dark
        }
    };

    let mut y: isize = -1;
    while y < (total_h as isize - 1) {
        let mut row = String::with_capacity(total_w);
        for x in -1..(width as isize + 1) {
            let top = is_dark(x, y);
            let bot = is_dark(x, y + 1);
            let ch = match (top, bot) {
                (false, false) => '█',
                (true, true) => ' ',
                (true, false) => '▄',
                (false, true) => '▀',
            };
            row.push(ch);
        }
        lines.push(row);
        y += 2;
    }
    lines
}

async fn send_mpv_ipc(sock_path: &str, json_line: &str) -> Result<(), String> {
    #[cfg(unix)]
    let mut stream: IpcStream = tokio::net::UnixStream::connect(sock_path)
        .await
        .map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let mut stream: IpcStream = tokio::net::windows::named_pipe::ClientOptions::new()
        .open(sock_path)
        .map_err(|e| e.to_string())?;
    #[cfg(not(any(unix, windows)))]
    return Err("IPC unsupported".to_string());

    #[cfg(any(unix, windows))]
    {
        stream
            .write_all(format!("{}\n", json_line).as_bytes())
            .await
            .map_err(|e| e.to_string())
    }
}

async fn apply_action(
    action: RemoteAction,
    state: &Arc<RwLock<RemotePlaybackState>>,
    active_sock: &Arc<RwLock<Option<String>>>,
    state_tx: &broadcast::Sender<String>,
) {
    let sock_opt = active_sock.read().await.clone();
    match action {
        RemoteAction::TogglePause => {
            if let Some(ref s) = sock_opt {
                let _ = send_mpv_ipc(s, r#"{"command":["cycle","pause"]}"#).await;
            }
            let mut st = state.write().await;
            st.is_playing = !st.is_playing;
        }
        RemoteAction::Seek { seconds } => {
            if let Some(ref s) = sock_opt {
                let _ = send_mpv_ipc(
                    s,
                    &format!(r#"{{"command":["seek",{:.1},"relative"]}}"#, seconds),
                )
                .await;
            }
        }
        RemoteAction::SkipForward { seconds } => {
            if let Some(ref s) = sock_opt {
                let _ = send_mpv_ipc(
                    s,
                    &format!(r#"{{"command":["seek",{:.1},"relative"]}}"#, seconds),
                )
                .await;
            }
        }
        RemoteAction::SetVolume { delta } => {
            let new_vol = {
                let mut st = state.write().await;
                st.volume = (st.volume + delta).clamp(0, 130);
                st.volume
            };
            if let Some(ref s) = sock_opt {
                let _ = send_mpv_ipc(
                    s,
                    &format!(r#"{{"command":["set_property","volume",{}]}}"#, new_vol),
                )
                .await;
            }
        }
        RemoteAction::SetSpeed { speed } => {
            let clamped = speed.clamp(0.25, 3.0);
            if let Some(ref s) = sock_opt {
                let _ = send_mpv_ipc(
                    s,
                    &format!(r#"{{"command":["set_property","speed",{:.2}]}}"#, clamped),
                )
                .await;
            }
            state.write().await.speed = clamped;
        }
        RemoteAction::ToggleSub => {
            if let Some(ref s) = sock_opt {
                let _ = send_mpv_ipc(s, r#"{"command":["cycle","sub-visibility"]}"#).await;
            }
            let mut st = state.write().await;
            st.sub_active = !st.sub_active;
        }
    }

    if let Ok(json) = serde_json::to_string(&*state.read().await) {
        let _ = state_tx.send(json);
    }
}

const MOBILE_HTML: &str = r#"<!doctype html>
<html>
<head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Phaenna Remote</title>
<style>
body{margin:0;background:#0d1017;color:#e6edf3;font-family:system-ui,sans-serif;padding:20px;text-align:center}
.card{max-width:420px;margin:0 auto;background:#161b26;border:1px solid #2d3548;border-radius:16px;padding:20px}
h1{font-size:18px;margin:0 0 6px;color:#ff4757}.sub{color:#8b949e;font-size:13px;margin-bottom:18px}
.row{display:flex;gap:10px;justify-content:center;margin:10px 0}
button{flex:1;padding:14px;border:none;border-radius:10px;background:#242b3d;color:#fff;font-size:15px;font-weight:600;cursor:pointer}
button.primary{background:#ff4757;color:#000}
</style>
</head>
<body>
<div class="card">
<h1 id="title">Phaenna Remote</h1>
<div class="sub" id="chan">Connecting...</div>
<div class="row">
<button onclick="send({cmd:'seek',seconds:-10})">-10s</button>
<button class="primary" onclick="send({cmd:'toggle_pause'})">Play / Pause</button>
<button onclick="send({cmd:'seek',seconds:10})">+10s</button>
</div>
<div class="row">
<button onclick="send({cmd:'set_volume',delta:-10})">Vol -</button>
<button onclick="send({cmd:'toggle_sub'})">CC</button>
<button onclick="send({cmd:'set_volume',delta:10})">Vol +</button>
</div>
<div class="row">
<button onclick="send({cmd:'set_speed',speed:1.0})">1.0x</button>
<button onclick="send({cmd:'set_speed',speed:1.25})">1.25x</button>
<button onclick="send({cmd:'set_speed',speed:1.5})">1.5x</button>
<button onclick="send({cmd:'set_speed',speed:2.0})">2.0x</button>
</div>
</div>
<script>
const ws = new WebSocket((location.protocol==='https:'?'wss://':'ws://')+location.host+'/ws');
ws.onmessage = e => {
  const s = JSON.parse(e.data);
  document.getElementById('title').textContent = s.title || 'Idle';
  document.getElementById('chan').textContent = (s.channel||'') + ' · ' + Math.floor(s.time_pos||0) + 's / Vol ' + s.volume + '%';
};
function send(obj){ ws.readyState===1 && ws.send(JSON.stringify(obj)); }
</script>
</body>
</html>"#;

async fn handle_client(
    mut stream: TcpStream,
    state: Arc<RwLock<RemotePlaybackState>>,
    active_sock: Arc<RwLock<Option<String>>>,
    state_tx: broadcast::Sender<String>,
) {
    let mut peek_buf = [0u8; 1024];
    let Ok(n) = stream.peek(&mut peek_buf).await else {
        return;
    };
    let head = String::from_utf8_lossy(&peek_buf[..n]);
    let is_ws = head.to_ascii_lowercase().contains("upgrade: websocket");

    if is_ws {
        let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
            return;
        };
        let (mut ws_tx, mut ws_rx) = ws.split();
        let mut rx = state_tx.subscribe();
        if let Ok(init_json) = serde_json::to_string(&*state.read().await) {
            let _ = ws_tx.send(Message::Text(init_json.into())).await;
        }

        loop {
            tokio::select! {
                Ok(msg) = rx.recv() => {
                    if ws_tx.send(Message::Text(msg.into())).await.is_err() {
                        break;
                    }
                }
                incoming = ws_rx.next() => {
                    match incoming {
                        Some(Ok(Message::Text(txt))) => {
                            if let Ok(action) = serde_json::from_str::<RemoteAction>(&txt) {
                                apply_action(action, &state, &active_sock, &state_tx).await;
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        _ => {}
                    }
                }
            }
        }
    } else {
        let mut req_buf = [0u8; 2048];
        let _ = stream.read(&mut req_buf).await;
        let body = MOBILE_HTML.as_bytes();
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(resp.as_bytes()).await;
        let _ = stream.write_all(body).await;
    }
}

pub async fn start_remote_server(port: u16) -> Option<RemoteServerHandle> {
    let bind_addr: SocketAddr = format!("0.0.0.0:{}", port).parse().ok()?;
    let listener = TcpListener::bind(bind_addr).await.ok()?;
    let local_ip = get_local_ip();
    let url = format!("http://{}:{}", local_ip, port);
    let qr_lines = generate_terminal_qr(&url);
    let state = Arc::new(RwLock::new(RemotePlaybackState {
        volume: 100,
        speed: 1.0,
        ..Default::default()
    }));
    let active_sock = Arc::new(RwLock::new(None));
    let (state_tx, _) = broadcast::channel::<String>(32);

    let st_clone = state.clone();
    let sock_clone = active_sock.clone();
    let btx = state_tx.clone();
    tokio::spawn(async move {
        loop {
            if let Ok((stream, _)) = listener.accept().await {
                let s = st_clone.clone();
                let sk = sock_clone.clone();
                let t = btx.clone();
                tokio::spawn(async move {
                    handle_client(stream, s, sk, t).await;
                });
            }
        }
    });

    Some(RemoteServerHandle {
        url,
        qr_lines,
        state,
        active_sock,
        state_tx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_qr_generation() {
        let lines = generate_terminal_qr("http://192.168.1.50:8485");
        assert!(lines.len() >= 10);
        assert!(lines.iter().all(|l| l.chars().count() >= 20));
    }
}
