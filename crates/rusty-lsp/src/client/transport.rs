//! Messages to the server and back: writing them, matching each reply to the
//! request it answers by id, and the thread that reads the server for as long
//! as it runs.

use std::{
    io::{BufReader, Read},
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{self, Sender},
    },
    thread,
    time::Duration,
};

use serde_json::{Value, json};

use super::{Shared, dispatch::dispatch};
use crate::{
    error::{Error, Result},
    model::LspEvent,
    rpc,
};

/// How long a request may take before the caller is told rather than kept
/// waiting. Generous because a cold index answers slowly; callers that want to
/// retry — completion during startup — retry above this.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

impl Shared {
    fn write(&self, message: &Value) -> Result<()> {
        let mut writer = self.writer.lock().expect("lsp writer");
        rpc::write_message(&mut **writer, message).map_err(Error::Io)
    }

    pub(crate) fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.write(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    pub(super) fn respond(&self, id: Value, result: Value) -> Result<()> {
        self.write(&json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    pub(crate) fn request(&self, method: &str, params: Value) -> Result<Value> {
        self.request_within(method, params, REQUEST_TIMEOUT)
    }

    pub(super) fn request_within(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let gone = || Error::Exited {
            method: method.to_string(),
        };
        if !self.alive.load(Ordering::Acquire) {
            return Err(gone());
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().expect("lsp pending").insert(id, tx);
        // The reader may have ended between the check above and the insert,
        // after failing every waiter it could see; one registered after that
        // would never be told. Look again now that this one is registered.
        if !self.alive.load(Ordering::Acquire) {
            self.pending.lock().expect("lsp pending").remove(&id);
            return Err(gone());
        }

        if let Err(error) =
            self.write(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
        {
            self.pending.lock().expect("lsp pending").remove(&id);
            return Err(error);
        }

        match rx.recv_timeout(timeout) {
            Ok(Some(response)) => {
                if let Some(error) = response.get("error") {
                    Err(Error::Server {
                        method: method.to_string(),
                        message: error["message"]
                            .as_str()
                            .unwrap_or("unknown error")
                            .to_string(),
                    })
                } else {
                    Ok(response.get("result").cloned().unwrap_or(Value::Null))
                }
            }
            Ok(None) => Err(gone()),
            Err(_) => {
                self.pending.lock().expect("lsp pending").remove(&id);
                Err(Error::Timeout {
                    method: method.to_string(),
                })
            }
        }
    }

    /// The reader has stopped: every request still waiting learns so now
    /// rather than at its timeout, and every later one at once.
    fn reader_gone(&self) {
        self.alive.store(false, Ordering::Release);
        let waiters: Vec<Sender<Option<Value>>> = self
            .pending
            .lock()
            .expect("lsp pending")
            .drain()
            .map(|(_, waiter)| waiter)
            .collect();
        for waiter in waiters {
            let _ = waiter.send(None);
        }
    }
}

/// Read the server forever, feeding responses and diagnostics.
///
/// On the way out it fails every waiting request and says the server has
/// exited. It also drops its own reference to [`Shared`], which — once the
/// client is dropped too — closes the poke channel and the events channel:
/// the puller ends, and the consumer's `recv` returns `None`.
pub(super) fn pump(reader: Box<dyn Read + Send>, shared: Arc<Shared>) {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        // An Err is treated as EOF: a mangled frame means the stream has
        // drifted and every later byte would misparse anyway.
        while let Ok(Some(message)) = rpc::read_message(&mut reader) {
            dispatch(&shared, message);
        }
        shared.reader_gone();
        let _ = shared.events.send(LspEvent::Exited {});
    });
}
