use std::{collections::HashMap, sync::Arc};

use async_std::{
    channel::{self, bounded, Receiver, Sender},
    io,
    sync::{Mutex, RwLock},
    task,
};
use futures::prelude::*;
use io::stdout;

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
}

#[derive(Clone)]
pub struct OutputSwitch {
    inner: Arc<RwLock<OutputsInner>>,
}

impl OutputSwitch {
    pub fn println<S: Into<String>>(&self, for_input: OutputId, text: S) {
        let s: String = text.into() + "\n";
        let out = self.inner.clone();
        task::spawn(async move {
            if let Some(output) = out.read().await.outputs.get(&for_input) {
                let mut output = output.lock().await;
                output
                    .write_all(s.as_bytes())
                    .await
                    .unwrap_or_else(|e| error!("Error writing to output"));
                output
                    .flush()
                    .await
                    .unwrap_or_else(|e| error!("Error flushing to output"));
            }
        });
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
