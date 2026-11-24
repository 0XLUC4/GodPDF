use std::{sync::Arc, thread, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use iroh::{
    Endpoint, EndpointAddr, SecretKey,
    endpoint::{RecvStream, SendStream, presets},
};
use serde::{Deserialize, Serialize};
use tokio::{
    runtime::Builder,
    sync::{mpsc, watch},
    time::timeout,
};

pub(super) const MAX_PDF: usize = 64 * 1024 * 1024;
const MAX_CODE: usize = 8192;
const MAX_LABEL: usize = 256;
const SECRET_LEN: usize = 32;
const CODE_PREFIX: &str = "godpdf1_";
const ALPN: &[u8] = b"godpdf/snapshot/1";
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const RELAY_WAIT: Duration = Duration::from_secs(8);
const SETUP_TIMEOUT: Duration = Duration::from_secs(60);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(120);

pub(super) type Turn = [u8; 32];
pub(super) const INITIAL_TURN: Turn = [0; 32];

#[derive(Clone)]
pub(super) struct Revision {
    pub bytes: Arc<Vec<u8>>,
    pub turn: Turn,
    pub grant: Option<Turn>,
}

pub(super) enum Event {
    Invitation(String),
    Connected(String),
    Document(Arc<Vec<u8>>),
    Update(Revision),
    Failed(String),
}

pub(super) enum Start {
    Host { document: Arc<Vec<u8>> },
    Join { code: String },
}

type Channels = (watch::Sender<Option<Revision>>, mpsc::Receiver<Event>);
type SessionResult<T> = Result<T, &'static str>;

#[derive(Serialize, Deserialize)]
struct Invitation {
    address: EndpointAddr,
    secret: [u8; SECRET_LEN],
}

impl Invitation {
    fn encode(&self) -> SessionResult<String> {
        let json = serde_json::to_vec(self).map_err(|_| "Could not create invitation")?;
        let code = format!("{CODE_PREFIX}{}", URL_SAFE_NO_PAD.encode(json));
        if code.len() > MAX_CODE {
            return Err("Could not create invitation");
        }
        Ok(code)
    }

    fn decode(code: &str) -> SessionResult<Self> {
        const INVALID: &str = "Invalid or unsupported invitation";
        if code.len() > MAX_CODE {
            return Err(INVALID);
        }
        let payload = code.trim().strip_prefix(CODE_PREFIX).ok_or(INVALID)?;
        let json = URL_SAFE_NO_PAD.decode(payload).map_err(|_| INVALID)?;
        serde_json::from_slice(&json).map_err(|_| INVALID)
    }
}

enum Session {
    Host(Arc<Vec<u8>>),
    Join(Invitation),
}

pub(super) fn start(mode: Start, peer_label: String) -> Result<Channels, String> {
    start_worker(mode, peer_label, false)
}

fn start_worker(mode: Start, label: String, loopback: bool) -> Result<Channels, String> {
    validate_label(&label)?;
    let session = match mode {
        Start::Host { document } => {
            validate_pdf_size(document.len())?;
            Session::Host(document)
        }
        Start::Join { code } => Session::Join(Invitation::decode(&code)?),
    };
    let (outgoing, revisions) = watch::channel(None);
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
                    result = run(session, label, revisions, &events, loopback) => result,
                };
                if let Err(message) = result {
                    let _ = events.try_send(Event::Failed(message.into()));
                }
            });
        })
        .map_err(|_| "Could not start collaboration thread".to_owned())?;
    Ok((outgoing, incoming))
}

async fn run(
    session: Session,
    label: String,
    revisions: watch::Receiver<Option<Revision>>,
    events: &mpsc::Sender<Event>,
    loopback: bool,
) -> SessionResult<()> {
    let builder = if loopback {
        Endpoint::builder(presets::Minimal).relay_mode(iroh::RelayMode::Disabled)
    } else {
        Endpoint::builder(presets::N0)
    };
    let endpoint = builder
        .secret_key(SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .map_err(|_| "Could not bind collaboration endpoint")?;

    // Keep the connection alive for both stream futures.
    let (_connection, send, recv) = match session {
        Session::Host(document) => {
            let address = if loopback {
                let port = endpoint
                    .bound_sockets()
                    .into_iter()
                    .find(|addr| addr.is_ipv4())
                    .ok_or("Could not bind loopback endpoint")?
                    .port();
                EndpointAddr::new(endpoint.id()).with_ip_addr(([127, 0, 0, 1], port).into())
            } else {
                // Direct addresses still work on the same network if the relay is offline.
                let _ = timeout(RELAY_WAIT, endpoint.online()).await;
                let address = endpoint.addr();
                if address.addrs.is_empty() {
                    return Err("No address available for the shared session");
                }
                address
            };
            let mut secret = [0; SECRET_LEN];
            getrandom::fill(&mut secret).map_err(|_| "Could not create invitation")?;
            publish(
                events,
                Event::Invitation(Invitation { address, secret }.encode()?),
            )
            .await?;

            let (connection, mut send, mut recv, guest_label) = loop {
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
                    break authenticated;
                }
            };

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
            (connection, send, recv)
        }
        Session::Join(invitation) => {
            let connection = timeout(SETUP_TIMEOUT, endpoint.connect(invitation.address, ALPN))
                .await
                .map_err(|_| "Connection timed out")?
                .map_err(|_| "Could not connect to host")?;
            let (send, recv, host_label, document) = timeout(TRANSFER_TIMEOUT, async {
                let (mut send, mut recv) = connection
                    .open_bi()
                    .await
                    .map_err(|_| "Could not open collaboration stream")?;
                write_bytes(&mut send, &invitation.secret).await?;
                write_label(&mut send, &label).await?;
                let host_label = read_label(&mut recv).await?;
                let document = Arc::new(read_frame(&mut recv, MAX_PDF).await?);
                write_bytes(&mut send, &[1]).await?;
                Ok::<_, &'static str>((send, recv, host_label, document))
            })
            .await
            .map_err(|_| "Initial snapshot transfer timed out")??;
            publish(events, Event::Document(document)).await?;
            publish(events, Event::Connected(host_label)).await?;
            (connection, send, recv)
        }
    };

    tokio::select! {
        result = read_revisions(recv, events) => result,
        result = write_revisions(send, revisions) => result,
    }
}

async fn publish(events: &mpsc::Sender<Event>, event: Event) -> SessionResult<()> {
    events
        .send(event)
        .await
        .map_err(|_| "Collaboration receiver closed")
}

async fn read_revisions(mut recv: RecvStream, events: &mpsc::Sender<Event>) -> SessionResult<()> {
    loop {
        let flag = read_byte(&mut recv).await?;
        let revision = timeout(TRANSFER_TIMEOUT, async {
            let turn = read_turn(&mut recv).await?;
            let grant = match flag {
                0 => None,
                1 => Some(read_turn(&mut recv).await?),
                _ => return Err("Invalid revision flag"),
            };
            let bytes = Arc::new(read_frame(&mut recv, MAX_PDF).await?);
            Ok(Revision { bytes, turn, grant })
        })
        .await
        .map_err(|_| "Revision transfer timed out")??;
        publish(events, Event::Update(revision)).await?;
    }
}

async fn write_revisions(
    mut send: SendStream,
    mut revisions: watch::Receiver<Option<Revision>>,
) -> SessionResult<()> {
    while revisions.changed().await.is_ok() {
        // Release the watch borrow before awaiting stream I/O.
        let revision = revisions.borrow_and_update().clone();
        if let Some(revision) = revision {
            validate_pdf_size(revision.bytes.len())?;
            timeout(TRANSFER_TIMEOUT, async {
                write_bytes(&mut send, &[u8::from(revision.grant.is_some())]).await?;
                write_bytes(&mut send, &revision.turn).await?;
                if let Some(turn) = revision.grant {
                    write_bytes(&mut send, &turn).await?;
                }
                write_frame(&mut send, &revision.bytes, MAX_PDF).await
            })
            .await
            .map_err(|_| "Revision transfer timed out")??;
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
        let document = Arc::new(godpdf_core::blank_document(72.0, 72.0).unwrap());
        let (host, mut host_events) = start_worker(
            Start::Host {
                document: document.clone(),
            },
            "Host".into(),
            true,
        )
        .unwrap();
        let Some(Event::Invitation(code)) = next(&mut host_events).await else {
            panic!("expected invitation");
        };
        let mut invalid = Invitation::decode(&code).unwrap();
        invalid.secret[0] ^= 1;
        let (rejected, mut rejected_events) = start_worker(
            Start::Join {
                code: invalid.encode().unwrap(),
            },
            "Uninvited guest".into(),
            true,
        )
        .unwrap();
        assert!(matches!(
            next(&mut rejected_events).await,
            Some(Event::Failed(_)) | None
        ));
        drop(rejected);

        let (guest, mut guest_events) =
            start_worker(Start::Join { code }, "Guest".into(), true).unwrap();
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
            [(&host, &mut guest_events, 1), (&guest, &mut host_events, 2)]
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
        drop(guest);
        assert!(next(&mut guest_events).await.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sender_drop_cancels_pending_accept() {
        let (host, mut events) = start_worker(
            Start::Host {
                document: Arc::new(vec![1]),
            },
            "Host".into(),
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
}
