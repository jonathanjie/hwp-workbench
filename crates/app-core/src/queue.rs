//! FIFO dispatch reserves order before work enters a thread pool.
use crate::{Backend, Error, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

pub type Response = mpsc::Receiver<Result<Value>>;
enum Action {
    Invoke(String, Value),
    Open(PathBuf),
    Close,
}
struct Request {
    action: Action,
    response: mpsc::SyncSender<Result<Value>>,
}
#[derive(Clone)]
pub struct CommandQueue {
    sender: mpsc::Sender<Request>,
    accepting: Arc<AtomicBool>,
}
impl CommandQueue {
    pub fn new(backend: Arc<Backend>) -> Self {
        let (sender, receiver) = mpsc::channel::<Request>();
        let accepting = Arc::new(AtomicBool::new(true));
        let worker_accepting = accepting.clone();
        std::thread::spawn(move || {
            while let Ok(request) = receiver.recv() {
                if !worker_accepting.load(Ordering::SeqCst) {
                    let _ = request
                        .response
                        .send(Err(Error::new("Desktop queue closed")));
                    continue;
                }
                let is_close = matches!(&request.action, Action::Close);
                let result = match request.action {
                    Action::Invoke(command, args) => backend.invoke(&command, args),
                    Action::Open(file) => backend.queue_open(file).map(|()| Value::Null),
                    Action::Close => backend.confirm_close().map(Value::Bool),
                };
                if is_close && matches!(&result, Ok(Value::Bool(true))) {
                    worker_accepting.store(false, Ordering::SeqCst);
                    backend.stop();
                }
                let _ = request.response.send(result);
            }
            backend.stop();
        });
        Self { sender, accepting }
    }
    fn enqueue(&self, action: Action) -> Result<Response> {
        if !self.is_open() {
            return Err(Error::new("Desktop queue closed"));
        }
        let (response, receiver) = mpsc::sync_channel(1);
        self.sender
            .send(Request { action, response })
            .map_err(|_| Error::new("Desktop queue unavailable"))?;
        Ok(receiver)
    }
    pub fn is_open(&self) -> bool {
        self.accepting.load(Ordering::SeqCst)
    }
    pub fn invoke(&self, command: &str, args: Value) -> Result<Response> {
        crate::check_arguments(command, &args)?;
        self.enqueue(Action::Invoke(command.into(), args))
    }
    pub fn open(&self, path: PathBuf) -> Result<Response> {
        self.enqueue(Action::Open(path))
    }
    pub fn close(&self) -> Result<Response> {
        self.enqueue(Action::Close)
    }
}
