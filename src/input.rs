use std::{collections::HashMap, fmt::Debug, sync::Arc};

use async_std::{
    channel::{bounded, Receiver, Sender},
    io,
    sync::{Mutex, RwLock},
    task,
};
use futures::prelude::*;
use libp2p::{kad::QueryId, request_response::RequestId};

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
    pending: HashMap<Pending, OutputId>,
}

#[derive(Clone)]
pub struct OutputSwitch {
    inner: Arc<RwLock<OutputsInner>>,
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
    pub fn println<S: Into<String>>(&self, for_input: OutputId, msg: S) {
        let s: String = msg.into() + "\n";
        let out = self.inner.clone();
        task::spawn(async move {
            if let Some(output) = out.read().await.outputs.get(&for_input) {
                write_msg!(output, s);
            }
        });
    }

    pub fn reply<S, P>(&self, for_pending: P, msg: S)
    where
        S: Into<String>,
        P: Into<Pending> + Debug,
    {
        let s: String = msg.into() + "\n";
        let out = self.inner.clone();
        let for_pending = for_pending.into();
        task::spawn(async move {
            let mut out = out.write().await;

            if let Some(output_id) = out.pending.remove(&for_pending) {
                if let Some(output) = out.outputs.get(&output_id) {
                    write_msg!(output, s);
                }
            } else {
                error!("Pending {:?} was not registered for output", for_pending);
            }
        });
    }

    pub fn send_to_all(&self, msg: impl Into<String>) {
        let msg: String = msg.into();
        let out = self.inner.clone();
        task::spawn(async move {
            for output in out.read().await.outputs.values() {
                let s = msg.as_str();
                write_msg!(output, s);
            }
        });
    }

    pub async fn aprintln<S: Into<String>>(&self, for_input: OutputId, text: S) {
        let s: String = text.into() + "\n";

        if let Some(output) = self.inner.read().await.outputs.get(&for_input) {
            write_msg!(output, s);
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
        if let Some(prev) = self
            .inner
            .write()
            .await
            .pending
            .insert(pending_id.into(), output_id)
        {
            error!("For same id previous record was pending")
        };
    }

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
    input_tx: Sender<Cmd>,
    outputs: OutputSwitch,
}

impl InputOutputSwitch {
    pub fn new(use_stdin: bool) -> Self {
        let (tx, rx) = bounded(1000);
        let mut outputs = HashMap::new();

        // Forwards standard input to input channel
        if use_stdin {
            let mut input = io::BufReader::new(io::stdin()).lines();

            let input_id = OutputId::from(0);
            let output: Output = Box::new(io::stdout());
            outputs.insert(input_id, Mutex::new(output));
            let sender = tx.clone();
            task::spawn(async move {
                while let Some(cmd) = input.next().await {
                    match cmd {
                        Ok(cmd) => {
                            if let Err(e) = sender.send((cmd, input_id)).await {
                                error!("Input channel is closed: {}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Std input read error: {}", e);
                            break;
                        }
                    }
                }
            });
        }

        let outputs = OutputSwitch {
            inner: Arc::new(RwLock::new(OutputsInner {
                outputs,
                counter: 0,
                pending: HashMap::new(),
            })),
        };

        InputOutputSwitch {
            input_rx: rx,
            input_tx: tx,
            outputs,
        }
    }

    pub async fn next(&mut self) -> Option<Cmd> {
        self.input_rx.next().await
    }

    pub fn outputs(&self) -> OutputSwitch {
        self.outputs.clone()
    }
}
