use async_trait::async_trait;
use futures::channel::{mpsc, oneshot};
use futures::stream::{self, StreamExt};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::marker::{Send, Unpin};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, RwLock};

type Dispatchers = Arc<RwLock<HashMap<Road, Vec<mpsc::Sender<Ticket>>>>>;
type CameraReadings = Arc<Mutex<HashMap<(Road, Plate), BTreeMap<Timestamp, MilePos>>>>;
type PendingTickets = Arc<Mutex<HashMap<Road, Vec<Ticket>>>>;
type GivenTickets = Arc<Mutex<HashMap<Plate, HashSet<Timestamp>>>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("[::]:18534").await?;
    let dispatchers = Dispatchers::default();
    let camera_readings = CameraReadings::default();
    let pending_tickets = PendingTickets::default();
    let given_tickets = GivenTickets::default();

    loop {
        let (mut stream, _) = listener.accept().await?;

        let (mut tcp_rx, tcp_tx) = stream.into_split();
        let mut tcp_rx = BufReader::new(tcp_rx);
        let mut tcp_tx = BufWriter::new(tcp_tx);
        match ClientMsg::read_from(&mut tcp_rx).await {
            Err(e) => {
                eprintln!("tcp read error: {e}");
            }
            Ok(ClientMsg::IAmCamera { road, mile, limit }) => {
                camera_connection(
                    tcp_tx,
                    tcp_rx,
                    road,
                    mile,
                    limit,
                    dispatchers.clone(),
                    camera_readings.clone(),
                    pending_tickets.clone(),
                    given_tickets.clone(),
                )
                .await
            }
            Ok(ClientMsg::IAmDispatcher { roads }) => {
                dispatcher_connection(
                    tcp_tx,
                    tcp_rx,
                    roads,
                    dispatchers.clone(),
                    pending_tickets.clone(),
                )
                .await
            }
            Ok(_) => {
                if let Err(e) = {
                    ErrorMsg {
                        msg: "expected introduction".into(),
                    }
                    .write_to(&mut tcp_tx)
                    .await
                } {
                    eprintln!("tcp write error: {e}");
                }
            }
        }
    }
}
async fn camera_connection(
    tcp_tx: impl AsyncWriteExt + Send + Unpin,
    tcp_rx: impl AsyncReadExt + Unpin + Send,
    road: Road,
    mile: MilePos,
    limit: Speed,
    dispatchers: Dispatchers,
    camera_readings: CameraReadings,
    pending_tickets: PendingTickets,
    given_tickets: GivenTickets,
) {
}
async fn dispatcher_connection(
    mut tcp_tx: impl AsyncWriteExt + Send + Unpin,
    mut tcp_rx: impl AsyncReadExt + Unpin + Send,
    roads: Vec<Road>,
    dispatchers: Dispatchers,
    pending_tickets: PendingTickets,
) {
    let (tx, mut rx) = mpsc::channel(100);
    {
        let mut dispatchers = dispatchers.write().await;
        for road in &roads {
            match dispatchers.entry(*road) {
                std::collections::hash_map::Entry::Occupied(mut occ) => {
                    occ.get_mut().push(tx.clone())
                }
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert(vec![tx.clone()]);
                }
            }
        }
    }
    let mut unsent_tickets = vec![];
    {
        let mut pending_tickets = pending_tickets.lock().await;
        for road in &roads {
            if let Some(mut vals) = pending_tickets.remove(&road) {
                unsent_tickets.append(&mut vals);
            }
        }
    }
    for ticket in unsent_tickets {
        if let Err(e) = ticket.write_to(&mut tcp_tx).await {
            eprintln!("write error in dispatcher_connection: {e}");
            return;
        }
    }

    let tickets = rx.map(|v| ControlFlow::Ticket(v));
    let messages = stream::unfold(tcp_rx, |mut tcp_rx| async move {
        match ClientMsg::read_from(&mut tcp_rx).await {
            Err(e) => Some((ControlFlow::Err(format!("{e}")), tcp_rx)),
            Ok(msg) => Some((ControlFlow::Msg(msg), tcp_rx)),
        }
    });
    let (heartbeats, heart_tx) = heartbeat_stream();
    let mut events = futures::stream_select!(tickets, messages, heartbeats);

    //main event loop
    tokio::spawn(async move { while let Some(event) = events.next().await {} });
}

enum ControlFlow {
    Ticket(Ticket),
    Msg(ClientMsg),
    Err(String),
    Disconnect,
    HeartBeat,
}
fn heartbeat_stream() -> (
    impl stream::Stream<Item = ControlFlow>,
    oneshot::Sender<Duration>,
) {
    use tokio::time::{interval, Interval};
    enum HeartbeatState {
        Dormant(oneshot::Receiver<Duration>),
        Beating(Interval),
        Dead,
    }
    let (heart_tx, heart_rx) = oneshot::channel();
    (
        stream::unfold(HeartbeatState::Dormant(heart_rx), |state| async move {
            match state {
                HeartbeatState::Dormant(tx) => match tx.await {
                    Ok(dur) => {
                        let mut inter = interval(dur);
                        inter.tick().await;
                        Some((ControlFlow::HeartBeat, HeartbeatState::Beating(inter)))
                    }
                    Err(e) => Some((
                        ControlFlow::Err(format!("heartbeat error: {e}")),
                        HeartbeatState::Dead,
                    )),
                },
                HeartbeatState::Beating(mut inter) => {
                    inter.tick().await;
                    Some((ControlFlow::HeartBeat, HeartbeatState::Beating(inter)))
                }
                HeartbeatState::Dead => Some((
                    ControlFlow::Err("heartbeat is dead".into()),
                    HeartbeatState::Dead,
                )),
            }
        }),
        heart_tx,
    )
}
#[async_trait]
trait WriteMsg {
    async fn write_to(
        &self,
        writer: &mut (impl AsyncWriteExt + Send + Unpin),
    ) -> Result<(), tokio::io::Error>;
}
#[async_trait]
trait ReadMsg {
    async fn read_from(
        reader: &mut (impl AsyncReadExt + Unpin + Send),
    ) -> Result<Self, tokio::io::Error>
    where
        Self: Sized;
}
#[async_trait]
impl ReadMsg for String {
    async fn read_from(
        reader: &mut (impl AsyncReadExt + Unpin + Send),
    ) -> Result<Self, tokio::io::Error>
    where
        Self: Sized,
    {
        let size = reader.read_u8().await?;
        let mut str_bytes = vec![0; size as usize];
        reader.read_exact(&mut str_bytes).await?;
        Ok(String::from_utf8_lossy(&str_bytes).into())
    }
}
#[async_trait]
impl WriteMsg for String {
    async fn write_to(
        &self,
        writer: &mut (impl AsyncWriteExt + Send + Unpin),
    ) -> Result<(), tokio::io::Error> {
        writer.write_u8(self.len() as u8).await?;
        writer.write_all(self.as_bytes()).await?;
        Ok(())
    }
}
enum ClientMsg {
    Plate {
        id: Plate,
        timestamp: Timestamp,
    },
    WantHeartBeat {
        interval: u32,
    },
    IAmCamera {
        road: Road,
        mile: MilePos,
        limit: Speed,
    },
    IAmDispatcher {
        roads: Vec<Road>,
    },
    UnknownMsg,
}
#[async_trait]
impl ReadMsg for ClientMsg {
    async fn read_from(
        reader: &mut (impl AsyncReadExt + Unpin + Send),
    ) -> Result<Self, tokio::io::Error> {
        match reader.read_u8().await? {
            0x20 => Ok(Self::Plate {
                id: String::read_from(reader).await?,
                timestamp: reader.read_u32().await?,
            }),
            0x40 => Ok(Self::WantHeartBeat {
                interval: reader.read_u32().await?,
            }),
            0x80 => Ok(Self::IAmCamera {
                road: reader.read_u16().await?,
                mile: reader.read_u16().await?,
                limit: reader.read_u16().await?,
            }),
            0x81 => {
                let num_roads = reader.read_u8().await?;
                let mut roads = Vec::with_capacity(num_roads as usize);
                for _ in 0..num_roads {
                    roads.push(reader.read_u16().await?);
                }
                Ok(Self::IAmDispatcher { roads })
            }
            _ => Ok(Self::UnknownMsg),
        }
    }
}

struct ErrorMsg {
    msg: String,
}
#[async_trait]
impl WriteMsg for ErrorMsg {
    async fn write_to(
        &self,
        writer: &mut (impl AsyncWriteExt + Send + Unpin),
    ) -> Result<(), tokio::io::Error> {
        writer.write_u8(0x10).await?;
        self.msg.write_to(writer).await?;
        writer.flush().await?;
        Ok(())
    }
}
struct Ticket {
    plate: Plate,
    road: Road,
    mile1: MilePos,
    timestamp1: Timestamp,
    mile2: MilePos,
    timestamp2: Timestamp,
    speed: Speed,
}
#[async_trait]
impl WriteMsg for Ticket {
    async fn write_to(
        &self,
        writer: &mut (impl AsyncWriteExt + Send + Unpin),
    ) -> Result<(), tokio::io::Error> {
        writer.write_u8(0x21).await?;
        self.plate.write_to(writer).await?;
        writer.write_u16(self.road).await?;
        writer.write_u16(self.mile1).await?;
        writer.write_u32(self.timestamp1).await?;
        writer.write_u16(self.mile2).await?;
        writer.write_u32(self.timestamp2).await?;
        writer.write_u16(self.speed).await?;

        writer.flush().await?;
        Ok(())
    }
}
struct Heartbeat;
#[async_trait]
impl WriteMsg for Heartbeat {
    async fn write_to(
        &self,
        writer: &mut (impl AsyncWriteExt + Send + Unpin),
    ) -> Result<(), tokio::io::Error> {
        writer.write_u8(0x41).await?;
        Ok(())
    }
}
type MilePos = u16;
type Timestamp = u32;
type Plate = String;
type Road = u16;
type Speed = u16;
