use async_std::{
    channel::{self, Receiver},
    io,
    net::TcpListener,
    sync::{Mutex, RwLock},
    task,
};
use channel::Sender;
use error::Result;
use futures::{future::Either, prelude::*};
use libp2p::{kad::QueryId, request_response::RequestId};
use std::{
    collections::HashMap,
    fmt::Debug,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::error;

const INPUT_CHANNEL_SIZE: usize = 1024;
const OUTPUT_CHANNEL_SIZE: usize = 1024;
const PENDING_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Pending {
    KadQuery(QueryId),
    RequestResponse(RequestId),
}

impl From<QueryId> for Pending {
    fn from(id: QueryId) -> Self {
        Pending::KadQuery(id)
    }
}

impl From<RequestId> for Pending {
    fn from(id: RequestId) -> Self {
        Pending::RequestResponse(id)
    }
}

enum OutputMsg {
    One { msg: String, output: OutputId },
    All { msg: String },
    Reply { msg: String, pending: Pending },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct OutputId(u64);

impl From<u64> for OutputId {
    fn from(n: u64) -> Self {
        OutputId(n)
    }
}

type Output = Box<dyn AsyncWrite + Send + Unpin>;
pub type Cmd = (String, OutputId);

struct OutputsInner {
    outputs: HashMap<OutputId, Mutex<Output>>,
    counter: u64,
    pending: HashMap<Pending, (OutputId, Instant)>,
}

#[derive(Clone)]
pub struct OutputSwitch {
    inner: Arc<RwLock<OutputsInner>>,
    sender: Sender<OutputMsg>,
}

macro_rules! write_msg {
    ($output: ident, $msg: expr) => {{
        let mut output = $output.lock().await;
        output
            .write_all($msg.as_bytes())
            .await
            .unwrap_or_else(|_e| error!("Error writing to output"));
        output
            .flush()
            .await
            .unwrap_or_else(|_e| error!("Error flushing to output"));
    }};
}

impl OutputSwitch {
    pub fn println<S: Into<String>>(&self, output: OutputId, msg: S) {
        let msg: String = msg.into() + "\n";
        if let Err(e) = self.sender.try_send(OutputMsg::One { msg, output }) {
            error!("Output send error {}", e);
        }
    }

    pub fn reply<S, P>(&self, for_pending: P, msg: S)
    where
        S: Into<String>,
        P: Into<Pending> + Debug,
    {
        let msg: String = msg.into() + "\n";
        let pending = for_pending.into();
        if let Err(e) = self.sender.try_send(OutputMsg::Reply { msg, pending }) {
            error!("Output send error {}", e);
        }
    }

    pub fn send_to_all(&self, msg: impl Into<String>) {
        let msg: String = msg.into() + "\n";
        if let Err(e) = self.sender.try_send(OutputMsg::All { msg }) {
            error!("Output send error {}", e);
        }
    }

    pub async fn add_output(&mut self, output: Output) -> OutputId {
        let mut inner = self.inner.write().await;
        let id = OutputId::from(inner.counter);
        inner.counter += 1;
        inner.outputs.insert(id, Mutex::new(output));
        id
    }

    pub async fn remove_output(&self, output_id: OutputId) {
        self.inner.write().await.outputs.remove(&output_id);
    }

    pub async fn register_pending(&self, output_id: OutputId, pending_id: impl Into<Pending>) {
        if self
            .inner
            .write()
            .await
            .pending
            .insert(pending_id.into(), (output_id, Instant::now()))
            .is_some()
        {
            error!("For same id previous record was pending")
        };
    }
    #[allow(dead_code)]
    pub fn cancel_pending(&self, pending_id: impl Into<Pending>) {
        let pending_id = pending_id.into();
        let out = self.inner.clone();
        task::spawn(async move {
            out.write().await.pending.remove(&pending_id);
        });
    }
}

pub struct InputOutputSwitch {
    input_rx: Receiver<Cmd>,
    outputs: OutputSwitch,
}

impl InputOutputSwitch {
    pub async fn new(use_stdin: bool, control_socket: Option<SocketAddr>) -> Result<Self> {
        let (tx, rx) = channel::bounded(INPUT_CHANNEL_SIZE);
        let mut outputs = HashMap::new();

        // macro to fwd commands
        macro_rules! input_loop {
            ($input:ident, $sender:ident, $output_id:ident) => {
                while let Some(cmd) = $input.next().await {
                    match cmd {
                        Ok(cmd) => {
                            if let Err(e) = $sender.send((cmd, $output_id)).await {
                                error!("Input channel is closed: {}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Input read error: {}", e);
                            break;
                        }
                    }
                }
            };
        }

        // Forwards standard input to control channel
        if use_stdin {
            let mut input = io::BufReader::new(io::stdin()).lines();

            let output_id = OutputId::from(0);
            let output: Output = Box::new(io::stdout());
            outputs.insert(output_id, Mutex::new(output));
            let sender = tx.clone();
            task::spawn(async move { input_loop!(input, sender, output_id) });
        }

        let (output_sender, mut output_receiver) = channel::bounded(OUTPUT_CHANNEL_SIZE);

        let outputs = OutputSwitch {
            inner: Arc::new(RwLock::new(OutputsInner {
                outputs,
                counter: 0,
                pending: HashMap::new(),
            })),
            sender: output_sender,
        };

        let out = outputs.clone();
        task::spawn(async move {
            let mut timeout_check =
                async_std::stream::interval(Duration::from_secs(PENDING_TIMEOUT_SECS));

            loop {
                match future::select(output_receiver.next(), timeout_check.next()).await {
                    Either::Left((None, _)) => break,
                    Either::Left((Some(msg), _)) => match msg {
                        OutputMsg::One { msg, output } => {
                            if let Some(output) = out.inner.read().await.outputs.get(&output) {
                                write_msg!(output, msg);
                            }
                        }
                        OutputMsg::All { msg } => {
                            let msg = msg.as_str();
                            for output in out.inner.read().await.outputs.values() {
                                write_msg!(output, msg);
                            }
                        }
                        OutputMsg::Reply { msg, pending } => {
                            let mut out = out.inner.write().await;

                            if let Some((output_id, _)) = out.pending.remove(&pending) {
                                if let Some(output) = out.outputs.get(&output_id) {
                                    write_msg!(output, msg);
                                }
                            } else {
                                error!("Pending {:?} was not registered for output", pending);
                            }
                        }
                    },
                    Either::Right((_, _)) => {
                        debug!("Pending outputs cleanup");
                        let mut out = out.inner.write().await;
                        let now = Instant::now();
                        out.pending.retain(|_, (_, created)| {
                            now.duration_since(*created).as_secs() < PENDING_TIMEOUT_SECS
                        });
                    }
                }
            }
            error!("Output loop ended prematurely")
        });

        // Listens on control TCP socket and forwards commands to control channel
        if let Some(addr) = control_socket {
            let listener = TcpListener::bind(addr).await?;
            let sender = tx.clone();
            let mut outputs = outputs.clone();
            task::spawn(async move {
                let mut incoming = listener.incoming();
                while let Some(conn) = incoming.next().await {
                    match conn {
                        Ok(socket) => {
                            let client_addr = socket
                                .peer_addr()
                                .map(|a| a.to_string())
                                .unwrap_or_else(|_| "<UKNOWN>".into());
                            debug!("Client {} connected on control socket", client_addr);
                            let (input, output) = socket.split();
                            let output_id = outputs.add_output(Box::new(output) as Output).await;
                            let sender = sender.clone();
                            let outputs = outputs.clone();
                            task::spawn(async move {
                                let mut input = io::BufReader::new(input).lines();
                                input_loop!(input, sender, output_id);
                                outputs.remove_output(output_id).await;
                                debug!("Client {} disconnected from control socket", client_addr)
                            });
                        }
                        Err(e) => error!("Error on incomming control connection: {}", e),
                    }
                }
            });
        }

        Ok(InputOutputSwitch {
            input_rx: rx,
            outputs,
        })
    }

    pub async fn next(&mut self) -> Option<Cmd> {
        self.input_rx.next().await
    }

    pub fn outputs(&self) -> OutputSwitch {
        self.outputs.clone()
    }
}
