use std::{cell::RefCell, net::Ipv4Addr, sync::Arc, thread, time::Duration};

use iroh::{
    Endpoint, EndpointAddr, RelayMode, RelayUrl, SecretKey,
    address_lookup::MemoryLookup,
    endpoint::{RecvStream, SendStream, presets},
};
use sha2::{Digest, Sha256};
use tokio::{
    net::UdpSocket,
    runtime::Builder,
    sync::{mpsc, watch},
    time::{Instant, sleep, timeout},
};

pub(super) const MAX_PDF: usize = 64 * 1024 * 1024;
const CODE_DIGITS: usize = 6;
const MAX_LABEL: usize = 256;
const SECRET_LEN: usize = 32;
const ALPN: &[u8] = b"godpdf/snapshot/2";
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
/// The host's address record can take a few seconds to reach the DNS: the guest retries.
const SETUP_TIMEOUT: Duration = Duration::from_secs(45);
const RETRY_DELAY: Duration = Duration::from_millis(700);
const LAN_LOOKUP_TIMEOUT: Duration = Duration::from_millis(300);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(120);
/// Revision flags: a new editing turn follows; the body equals the last PDF exchanged.
const FLAG_GRANT: u8 = 1;
const FLAG_UNCHANGED: u8 = 2;

pub(super) type Turn = [u8; 32];
pub(super) const INITIAL_TURN: Turn = [0; 32];

#[derive(Clone)]
pub(super) struct Revision {
    pub bytes: Arc<Vec<u8>>,
    pub turn: Turn,
    pub grant: Option<Turn>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Cursor {
    pub page: u32,
    pub x: f32,
    pub y: f32,
}

impl Cursor {
    fn encode(self) -> [u8; 12] {
        let mut bytes = [0; 12];
        bytes[..4].copy_from_slice(&self.page.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.x.to_le_bytes());
        bytes[8..].copy_from_slice(&self.y.to_le_bytes());
        bytes
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let [a, b, c, d, e, f, g, h, i, j, k, l] = *bytes else {
            return None;
        };
        let x = f32::from_le_bytes([e, f, g, h]);
        let y = f32::from_le_bytes([i, j, k, l]);
        ((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)).then_some(Self {
            page: u32::from_le_bytes([a, b, c, d]),
            x,
            y,
        })
    }
}

pub(super) enum Event {
    Invitation(String),
    Connected(String),
    Document(Arc<Vec<u8>>),
    Update(Revision),
    Cursor(Option<Cursor>),
    Failed(String),
}

pub(super) enum Start {
    Host { document: Arc<Vec<u8>> },
    Join { code: String },
}

type Channels = (
    watch::Sender<Option<Revision>>,
    mpsc::Receiver<Event>,
    watch::Sender<Option<Cursor>>,
);
type SessionResult<T> = Result<T, &'static str>;

/// A random 6-digit code, e.g. "042917".
fn new_code() -> SessionResult<String> {
    const RANGE: u32 = 10u32.pow(CODE_DIGITS as u32);
    // Rejection sampling keeps every code equally likely.
    loop {
        let mut bytes = [0; 4];
        getrandom::fill(&mut bytes).map_err(|_| "Could not create invitation")?;
        let value = u32::from_le_bytes(bytes);
        if value < u32::MAX - u32::MAX % RANGE {
            return Ok(format!("{:0width$}", value % RANGE, width = CODE_DIGITS));
        }
    }
}

/// The digits of a typed or pasted code ("123 456", "123-456"), if it has exactly six.
pub(super) fn normalize_code(code: &str) -> Option<String> {
    let digits: String = code
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-')
        .collect();
    (digits.len() == CODE_DIGITS && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then_some(digits)
}

/// Both peers derive the host identity and the join secret from the code alone, so the
/// code is all a guest needs: iroh's DNS lookup finds the host by its public key.
fn derive(code: &str, purpose: &str) -> [u8; 32] {
    Sha256::new()
        .chain_update(b"godpdf/share/2/")
        .chain_update(purpose)
        .chain_update(b"/")
        .chain_update(code)
        .finalize()
        .into()
}

fn host_key(code: &str) -> SecretKey {
    SecretKey::from_bytes(&derive(code, "endpoint"))
}

fn discovery_port(probe: &[u8; 32]) -> u16 {
    40_000 + u16::from_le_bytes([probe[0], probe[1]]) % 9_000
}

async fn announce_on_lan(socket: UdpSocket, probe: [u8; 32], quic_port: u16) {
    let mut request = [0; 33];
    while let Ok((length, guest)) = socket.recv_from(&mut request).await {
        if request.get(..length) == Some(&probe[..]) {
            let _ = socket.send_to(&quic_port.to_le_bytes(), guest).await;
        }
    }
}

async fn find_on_lan(probe: &[u8; 32], host: &EndpointAddr) -> Option<EndpointAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.ok()?;
    socket.set_broadcast(true).ok()?;
    let port = discovery_port(probe);
    for address in [Ipv4Addr::BROADCAST, Ipv4Addr::LOCALHOST] {
        let _ = socket.send_to(probe, (address, port)).await;
    }
    let mut response = [0; 3];
    loop {
        let (length, source) = socket.recv_from(&mut response).await.ok()?;
        if length == 2 && source.port() == port {
            let quic_port = u16::from_le_bytes([response[0], response[1]]);
            if quic_port != 0 {
                return Some(host.clone().with_ip_addr((source.ip(), quic_port).into()));
            }
        }
    }
}

fn dedicated_relay(value: &str) -> SessionResult<RelayUrl> {
    let relay = value
        .trim()
        .parse::<RelayUrl>()
        .map_err(|_| "Invalid GodPDF relay URL")?;
    if relay.scheme() != "https" || relay.host().is_none() {
        return Err("The GodPDF relay must have a valid HTTPS URL");
    }
    Ok(relay)
}

enum Session {
    Host(Arc<Vec<u8>>),
    Join(String),
}

pub(super) fn start(mode: Start, peer_label: String) -> Result<Channels, String> {
    start_worker(mode, peer_label, None, true)
}

/// `local`, for tests: peers find each other in memory, on the loopback interface.
fn start_worker(
    mode: Start,
    label: String,
    local: Option<MemoryLookup>,
    use_lan: bool,
) -> Result<Channels, String> {
    validate_label(&label)?;
    let session = match mode {
        Start::Host { document } => {
            validate_pdf_size(document.len())?;
            Session::Host(document)
        }
        Start::Join { code } => {
            Session::Join(normalize_code(&code).ok_or("Enter the 6-digit code")?)
        }
    };
    let (outgoing, revisions) = watch::channel(None);
    let (cursor_sender, cursors) = watch::channel(None);
    let mut cancellation = revisions.clone();
    let (events, incoming) = mpsc::channel(4);
    thread::Builder::new()
        .name("godpdf-collaboration".into())
        .spawn(move || {
            let Ok(runtime) = Builder::new_current_thread().enable_all().build() else {
                let _ = events.try_send(Event::Failed(
                    "Could not start collaboration runtime".into(),
                ));
                return;
            };
            runtime.block_on(async {
                let result = tokio::select! {
                    biased;
                    _ = async { while cancellation.changed().await.is_ok() {} } => return,
                    _ = events.closed() => return,
                    result = run(session, label, revisions, cursors, &events, local, use_lan) => result,
                };
                if let Err(message) = result {
                    let _ = events.try_send(Event::Failed(message.into()));
                }
            });
        })
        .map_err(|_| "Could not start collaboration thread".to_owned())?;
    Ok((outgoing, incoming, cursor_sender))
}

async fn run(
    session: Session,
    label: String,
    mut revisions: watch::Receiver<Option<Revision>>,
    cursors: watch::Receiver<Option<Cursor>>,
    events: &mpsc::Sender<Event>,
    local: Option<MemoryLookup>,
    use_lan: bool,
) -> SessionResult<()> {
    let relay = if local.is_none() {
        std::env::var("GODPDF_RELAY_URL")
            .ok()
            .map(|value| dedicated_relay(&value))
            .transpose()?
    } else {
        None
    };
    let relay_mode = match &relay {
        Some(url) => RelayMode::custom([url.clone()]),
        None => iroh::endpoint::default_relay_mode(),
    };
    let builder = match &local {
        Some(lookup) => Endpoint::builder(presets::Minimal)
            .relay_mode(iroh::RelayMode::Disabled)
            .address_lookup(lookup.clone()),
        None if relay.is_some() => {
            Endpoint::builder(presets::Minimal).relay_mode(relay_mode.clone())
        }
        None => Endpoint::builder(presets::N0),
    };
    let (code, secret_key) = match &session {
        Session::Host(_) => {
            let code = new_code()?;
            let key = host_key(&code);
            (code, key)
        }
        Session::Join(code) => (code.clone(), SecretKey::generate()),
    };
    let endpoint = builder
        .secret_key(secret_key)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .map_err(|_| "Could not bind collaboration endpoint")?;
    let secret = derive(&code, "join");
    let probe = derive(&code, "lan-discovery");

    // Keep the connection alive for both stream futures.
    let (connection, send, recv, exchanged) = match session {
        Session::Host(document) => {
            let quic_port = endpoint
                .bound_sockets()
                .into_iter()
                .find(|addr| addr.is_ipv4())
                .map(|addr| addr.port());
            let discovery = match quic_port {
                Some(port) => UdpSocket::bind((Ipv4Addr::UNSPECIFIED, discovery_port(&probe)))
                    .await
                    .ok()
                    .map(|socket| (socket, port)),
                None => None,
            };
            publish(events, Event::Invitation(code)).await?;

            let accept = async {
                loop {
                    let incoming = endpoint
                        .accept()
                        .await
                        .ok_or("Collaboration endpoint closed")?;
                    let candidate = timeout(AUTH_TIMEOUT, async {
                        let connection = incoming.await.map_err(|_| "Connection failed")?;
                        let (send, mut recv) =
                            connection.accept_bi().await.map_err(|_| "Stream failed")?;
                        let mut supplied = [0; SECRET_LEN];
                        recv.read_exact(&mut supplied)
                            .await
                            .map_err(|_| "Invitation check failed")?;
                        if supplied != secret {
                            return Err("Invitation check failed");
                        }
                        let guest_label = read_label(&mut recv).await?;
                        Ok((connection, send, recv, guest_label))
                    })
                    .await;
                    if let Ok(Ok(authenticated)) = candidate {
                        break Ok::<_, &'static str>(authenticated);
                    }
                }
            };
            let (connection, mut send, mut recv, guest_label) = tokio::select! {
                result = accept => result?,
                _ = async {
                    if let Some((socket, port)) = discovery {
                        announce_on_lan(socket, probe, port).await;
                    }
                    std::future::pending::<()>().await;
                } => unreachable!(),
            };

            // The host can edit while waiting for the guest to enter the code.
            let document = revisions
                .borrow_and_update()
                .as_ref()
                .map(|revision| revision.bytes.clone())
                .unwrap_or(document);
            timeout(TRANSFER_TIMEOUT, async {
                write_label(&mut send, &label).await?;
                write_frame(&mut send, &document, MAX_PDF).await?;
                if read_byte(&mut recv).await? != 1 {
                    return Err("Invalid snapshot acknowledgement");
                }
                Ok(())
            })
            .await
            .map_err(|_| "Initial snapshot transfer timed out")??;
            publish(events, Event::Connected(guest_label)).await?;
            (connection, send, recv, document)
        }
        Session::Join(_) => {
            let deadline = Instant::now() + SETUP_TIMEOUT;
            let host = EndpointAddr::new(host_key(&code).public());
            let mut internet_target = host.clone();
            if local.is_none() {
                // Try the same relays as the host even if its DNS record is unavailable.
                let relays = relay_mode.relay_map();
                for relay in relays.urls::<Vec<_>>() {
                    internet_target = internet_target.with_relay_url(relay);
                }
            }
            let mut use_relay_hints = true;
            let connection = loop {
                let fallback = if use_relay_hints || relay.is_some() {
                    &internet_target
                } else {
                    &host
                };
                use_relay_hints = !use_relay_hints;
                let target = if use_lan {
                    timeout(LAN_LOOKUP_TIMEOUT, find_on_lan(&probe, &host))
                        .await
                        .ok()
                        .flatten()
                        .map(|lan| fallback.clone().with_addrs(lan.addrs))
                        .unwrap_or_else(|| fallback.clone())
                } else {
                    fallback.clone()
                };
                match timeout(AUTH_TIMEOUT, endpoint.connect(target, ALPN)).await {
                    Ok(Ok(connection)) => break connection,
                    _ if Instant::now() + RETRY_DELAY >= deadline => {
                        return Err("No shared PDF found for this code");
                    }
                    _ => sleep(RETRY_DELAY).await,
                }
            };
            let (send, recv, host_label, document) = timeout(TRANSFER_TIMEOUT, async {
                let (mut send, mut recv) = connection
                    .open_bi()
                    .await
                    .map_err(|_| "Could not open collaboration stream")?;
                write_bytes(&mut send, &secret).await?;
                write_label(&mut send, &label).await?;
                let host_label = read_label(&mut recv).await?;
                let document = Arc::new(read_frame(&mut recv, MAX_PDF).await?);
                write_bytes(&mut send, &[1]).await?;
                Ok::<_, &'static str>((send, recv, host_label, document))
            })
            .await
            .map_err(|_| "Initial snapshot transfer timed out")??;
            publish(events, Event::Document(document.clone())).await?;
            publish(events, Event::Connected(host_label)).await?;
            (connection, send, recv, document)
        }
    };

    // Turns alternate, so both peers agree on the last PDF exchanged in either direction.
    let exchanged = RefCell::new(exchanged);
    tokio::select! {
        result = read_revisions(recv, events, &exchanged) => result,
        result = write_revisions(send, revisions, &exchanged) => result,
        result = exchange_cursors(&connection, cursors, events) => result,
    }
}

async fn exchange_cursors(
    connection: &iroh::endpoint::Connection,
    outgoing: watch::Receiver<Option<Cursor>>,
    events: &mpsc::Sender<Event>,
) -> SessionResult<()> {
    // Datagram loss must neither block a PDF revision nor leave a stale pointer visible.
    let mut tick = tokio::time::interval(Duration::from_millis(40));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut latest = None;
    let mut last_packet = Instant::now();
    let mut last_sent = None;
    let mut last_send = Instant::now();
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let position = *outgoing.borrow();
                if position != last_sent
                    || (position.is_some() && last_send.elapsed() >= Duration::from_millis(250))
                {
                    let packet = position.map_or_else(|| vec![0], |cursor| cursor.encode().to_vec());
                    let _ = connection.send_datagram(packet.into());
                    last_sent = position;
                    last_send = Instant::now();
                }
                if latest.is_some() && last_packet.elapsed() > Duration::from_secs(1) {
                    latest = None;
                    publish(events, Event::Cursor(None)).await?;
                }
            }
            packet = connection.read_datagram() => {
                let packet = packet.map_err(|_| "Collaboration stream closed")?;
                let position = if packet.as_ref() == [0] {
                    None
                } else if let Some(position) = Cursor::decode(&packet) {
                    Some(position)
                } else {
                    continue;
                };
                last_packet = Instant::now();
                if latest != position {
                    latest = position;
                    publish(events, Event::Cursor(position)).await?;
                }
            }
        }
    }
}

async fn publish(events: &mpsc::Sender<Event>, event: Event) -> SessionResult<()> {
    events
        .send(event)
        .await
        .map_err(|_| "Collaboration receiver closed")
}

async fn read_revisions(
    mut recv: RecvStream,
    events: &mpsc::Sender<Event>,
    exchanged: &RefCell<Arc<Vec<u8>>>,
) -> SessionResult<()> {
    loop {
        let flags = read_byte(&mut recv).await?;
        if flags & !(FLAG_GRANT | FLAG_UNCHANGED) != 0 {
            return Err("Invalid revision flag");
        }
        let revision = timeout(TRANSFER_TIMEOUT, async {
            let turn = read_turn(&mut recv).await?;
            let grant = match flags & FLAG_GRANT {
                0 => None,
                _ => Some(read_turn(&mut recv).await?),
            };
            let bytes = if flags & FLAG_UNCHANGED != 0 {
                exchanged.borrow().clone()
            } else {
                Arc::new(read_frame(&mut recv, MAX_PDF).await?)
            };
            Ok::<_, &'static str>(Revision { bytes, turn, grant })
        })
        .await
        .map_err(|_| "Revision transfer timed out")??;
        *exchanged.borrow_mut() = revision.bytes.clone();
        publish(events, Event::Update(revision)).await?;
    }
}

async fn write_revisions(
    mut send: SendStream,
    mut revisions: watch::Receiver<Option<Revision>>,
    exchanged: &RefCell<Arc<Vec<u8>>>,
) -> SessionResult<()> {
    while revisions.changed().await.is_ok() {
        // Release the watch borrow before awaiting stream I/O.
        let revision = revisions.borrow_and_update().clone();
        if let Some(revision) = revision {
            validate_pdf_size(revision.bytes.len())?;
            // Passing the turn right after an edit resends only the flags, not the PDF.
            let unchanged = *exchanged.borrow() == revision.bytes;
            let mut flags = 0;
            if revision.grant.is_some() {
                flags |= FLAG_GRANT;
            }
            if unchanged {
                flags |= FLAG_UNCHANGED;
            }
            timeout(TRANSFER_TIMEOUT, async {
                write_bytes(&mut send, &[flags]).await?;
                write_bytes(&mut send, &revision.turn).await?;
                if let Some(turn) = revision.grant {
                    write_bytes(&mut send, &turn).await?;
                }
                if unchanged {
                    Ok(())
                } else {
                    write_frame(&mut send, &revision.bytes, MAX_PDF).await
                }
            })
            .await
            .map_err(|_| "Revision transfer timed out")??;
            *exchanged.borrow_mut() = revision.bytes;
        }
    }
    Ok(())
}

fn validate_pdf_size(len: usize) -> SessionResult<()> {
    if !(1..=MAX_PDF).contains(&len) {
        return Err("PDF snapshot size is out of range");
    }
    Ok(())
}

fn validate_label(label: &str) -> SessionResult<()> {
    if !(1..=MAX_LABEL).contains(&label.len()) {
        return Err("Peer label size is out of range");
    }
    Ok(())
}

async fn write_label(send: &mut SendStream, label: &str) -> SessionResult<()> {
    write_frame(send, label.as_bytes(), MAX_LABEL).await
}

async fn read_label(recv: &mut RecvStream) -> SessionResult<String> {
    String::from_utf8(read_frame(recv, MAX_LABEL).await?).map_err(|_| "Invalid peer label")
}

async fn write_bytes(send: &mut SendStream, bytes: &[u8]) -> SessionResult<()> {
    send.write_all(bytes)
        .await
        .map_err(|_| "Collaboration stream write failed")
}

async fn read_byte(recv: &mut RecvStream) -> SessionResult<u8> {
    let mut byte = [0];
    recv.read_exact(&mut byte)
        .await
        .map_err(|_| "Collaboration stream closed")?;
    Ok(byte[0])
}

async fn read_turn(recv: &mut RecvStream) -> SessionResult<Turn> {
    let mut turn = INITIAL_TURN;
    recv.read_exact(&mut turn)
        .await
        .map_err(|_| "Collaboration stream closed")?;
    Ok(turn)
}

async fn write_frame(send: &mut SendStream, bytes: &[u8], limit: usize) -> SessionResult<()> {
    if !(1..=limit).contains(&bytes.len()) {
        return Err("Collaboration message size is out of range");
    }
    write_bytes(send, &(bytes.len() as u32).to_be_bytes()).await?;
    write_bytes(send, bytes).await
}

async fn read_frame(recv: &mut RecvStream, limit: usize) -> SessionResult<Vec<u8>> {
    let mut header = [0; 4];
    recv.read_exact(&mut header)
        .await
        .map_err(|_| "Collaboration stream closed")?;
    let len = u32::from_be_bytes(header) as usize;
    if !(1..=limit).contains(&len) {
        return Err("Collaboration message size is out of range");
    }
    let mut bytes = vec![0; len];
    recv.read_exact(&mut bytes)
        .await
        .map_err(|_| "Collaboration stream closed")?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn next(events: &mut mpsc::Receiver<Event>) -> Option<Event> {
        timeout(Duration::from_secs(15), events.recv())
            .await
            .expect("timed out waiting for collaboration event")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn loopback_shares_snapshot_and_revisions() {
        let lookup = MemoryLookup::new();
        let document = Arc::new(godpdf_core::blank_document(72.0, 72.0).unwrap());
        let (host, mut host_events, host_cursor) = start_worker(
            Start::Host {
                document: document.clone(),
            },
            "Host".into(),
            Some(lookup.clone()),
            true,
        )
        .unwrap();
        let Some(Event::Invitation(code)) = next(&mut host_events).await else {
            panic!("expected invitation");
        };
        assert_eq!(normalize_code(&code).as_ref(), Some(&code));

        let document = Arc::new(godpdf_core::insert_blank_page(&document, 1, 72.0, 72.0).unwrap());
        host.send_replace(Some(Revision {
            bytes: document.clone(),
            turn: INITIAL_TURN,
            grant: None,
        }));
        let (guest, mut guest_events, guest_cursor) = start_worker(
            Start::Join {
                code: format!("{} {}", &code[..3], &code[3..]),
            },
            "Guest".into(),
            Some(MemoryLookup::new()),
            true,
        )
        .unwrap();
        assert!(
            matches!(next(&mut guest_events).await, Some(Event::Document(bytes)) if bytes == document)
        );
        assert!(
            matches!(next(&mut guest_events).await, Some(Event::Connected(label)) if label == "Host")
        );
        assert!(
            matches!(next(&mut host_events).await, Some(Event::Connected(label)) if label == "Guest")
        );

        let mut previous = document;
        let mut turn = INITIAL_TURN;
        for (sender, receiver, page) in
            [(&host, &mut guest_events, 2), (&guest, &mut host_events, 3)]
        {
            let bytes =
                Arc::new(godpdf_core::insert_blank_page(&previous, page, 72.0, 72.0).unwrap());
            let next_turn = [page as u8; 32];
            assert!(
                sender
                    .send(Some(Revision {
                        bytes: bytes.clone(),
                        turn,
                        grant: Some(next_turn),
                    }))
                    .is_ok()
            );
            assert!(
                matches!(next(receiver).await, Some(Event::Update(update)) if update.bytes == bytes && update.turn == turn && update.grant == Some(next_turn))
            );
            previous = bytes;
            turn = next_turn;
        }

        // Passing the turn without an edit sends no PDF body; the same PDF arrives.
        host.send(Some(Revision {
            bytes: previous.clone(),
            turn,
            grant: Some([9; 32]),
        }))
        .unwrap();
        assert!(
            matches!(next(&mut guest_events).await, Some(Event::Update(update)) if update.bytes == previous && update.grant == Some([9; 32]))
        );

        let host_position = Cursor {
            page: 1,
            x: 0.25,
            y: 0.75,
        };
        let guest_position = Cursor {
            page: 0,
            x: 0.75,
            y: 0.25,
        };
        host_cursor.send_replace(Some(host_position));
        guest_cursor.send_replace(Some(guest_position));
        assert!(
            matches!(next(&mut guest_events).await, Some(Event::Cursor(Some(position))) if position == host_position)
        );
        assert!(
            matches!(next(&mut host_events).await, Some(Event::Cursor(Some(position))) if position == guest_position)
        );
        guest_cursor.send_replace(None);
        assert!(matches!(
            next(&mut host_events).await,
            Some(Event::Cursor(None))
        ));

        drop(guest);
        assert!(next(&mut guest_events).await.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sender_drop_cancels_pending_accept() {
        let (host, mut events, _cursor) = start_worker(
            Start::Host {
                document: Arc::new(vec![1]),
            },
            "Host".into(),
            Some(MemoryLookup::new()),
            true,
        )
        .unwrap();
        assert!(matches!(
            next(&mut events).await,
            Some(Event::Invitation(_))
        ));
        drop(host);
        assert!(next(&mut events).await.is_none());
    }

    #[test]
    fn codes_are_six_digits_and_tolerate_separators() {
        let code = new_code().unwrap();
        assert_eq!(normalize_code(&code), Some(code));
        assert_eq!(normalize_code(" 123-456 "), Some("123456".into()));
        assert_eq!(normalize_code("12345"), None);
        assert_eq!(normalize_code("12345a"), None);
        assert_eq!(normalize_code("godpdf:123456"), None);
        assert_ne!(derive("123456", "endpoint"), derive("123456", "join"));
    }

    #[test]
    fn dedicated_relay_requires_https() {
        let relay = dedicated_relay("https://relay.example.com").unwrap();
        assert_eq!(relay.host_str(), Some("relay.example.com"));
        assert!(dedicated_relay("http://relay.example.com").is_err());
        assert!(dedicated_relay("relay.example.com").is_err());
    }

    #[test]
    fn invalid_codes_are_refused_before_connecting() {
        assert!(start(Start::Join { code: "abc".into() }, "Guest".into()).is_err());
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;

    /// Uses the production Iroh configuration, including relay discovery when available.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "needs internet"]
    async fn short_code_finds_host_without_lan_discovery() {
        let document = Arc::new(godpdf_core::blank_document(72.0, 72.0).unwrap());
        let started = std::time::Instant::now();
        let (_host, mut host_events, _host_cursor) = start(
            Start::Host {
                document: document.clone(),
            },
            "Host".into(),
        )
        .unwrap();
        let Some(Event::Invitation(code)) = host_events.recv().await else {
            panic!("expected invitation");
        };
        println!("invitation ready after {:?}", started.elapsed());
        let joined = std::time::Instant::now();
        let (_guest, mut guest_events, _guest_cursor) =
            start_worker(Start::Join { code }, "Guest".into(), None, false).unwrap();
        let event = timeout(Duration::from_secs(60), guest_events.recv())
            .await
            .unwrap();
        match event {
            Some(Event::Document(bytes)) => assert_eq!(bytes, document),
            Some(Event::Failed(error)) => panic!("{error}"),
            _ => panic!("unexpected event"),
        }
        println!("joined after {:?}", joined.elapsed());
    }
}
