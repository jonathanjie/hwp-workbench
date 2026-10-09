//! Nonblocking pipe transport for the macOS/Linux worker boundary.

use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use rustix::event::{poll, PollFd, PollFlags, Timespec};
use rustix::fs::{fcntl_getfl, fcntl_setfl, OFlags};
use serde_json::{json, Value};

use crate::{Error, WorkerCommand, CHUNK_SIZE, MAX_DOCUMENT, MAX_JSON};

pub(crate) struct Limits {
    pub timeout: Duration,
    pub max_output: usize,
}

// Always reap the direct child, including when setup or transport fails.
struct WorkerProcess(Child);

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn transport_error(error: impl std::fmt::Display) -> Error {
    Error::new("INDEPENDENT_FAILED", error.to_string())
}

fn timeout_error() -> Error {
    Error::new(
        "INDEPENDENT_TIMEOUT",
        "rhwp exceeded the 30-second timeout.",
    )
}

fn nonblocking(pipe: &impl AsFd) -> Result<(), Error> {
    let flags = fcntl_getfl(pipe).map_err(transport_error)?;
    fcntl_setfl(pipe, flags | OFlags::NONBLOCK).map_err(transport_error)
}

pub(crate) fn invoke_with_limits(
    payload: &[u8],
    op: &str,
    page: Option<u32>,
    format: Option<&str>,
    worker: &WorkerCommand,
    limits: Limits,
) -> Result<Value, Error> {
    if payload.is_empty() || payload.len() > MAX_DOCUMENT {
        return Err(Error::new(
            "LIMIT",
            "Independent parser input must be between 1 byte and 64 MiB.",
        ));
    }
    let mut request = json!({"op": op});
    if let Some(object) = request.as_object_mut() {
        if let Some(page) = page {
            object.insert("page".into(), json!(page));
        }
        if let Some(format) = format {
            object.insert("format".into(), json!(format));
        }
    }
    let encoded = serde_json::to_vec(&request).map_err(transport_error)?;
    if encoded.len() > MAX_JSON {
        return Err(Error::new("BAD_REQUEST", "Invalid JSON frame length."));
    }
    let length = u32::try_from(encoded.len()).map_err(transport_error)?;
    let mut frame = length.to_le_bytes().to_vec();
    frame.extend(encoded);
    frame.extend_from_slice(payload);

    let deadline = Instant::now()
        .checked_add(limits.timeout)
        .ok_or_else(timeout_error)?;
    let mut process = WorkerProcess(
        Command::new(&worker.program)
            .args(&worker.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(transport_error)?,
    );
    let output = communicate(&mut process.0, &frame, deadline, limits.max_output)?;
    let status = wait_until(&mut process.0, deadline)?;
    decode_response(&output, status)
}

#[derive(Clone, Copy)]
enum PipeKind {
    Input,
    Output,
    Error,
}

struct Pipes {
    input: Option<ChildStdin>,
    output: Option<ChildStdout>,
    error: Option<ChildStderr>,
}

impl Pipes {
    fn take(child: &mut Child) -> Result<Self, Error> {
        let missing = || transport_error("Worker pipe is unavailable.");
        let input = child.stdin.take().ok_or_else(missing)?;
        let output = child.stdout.take().ok_or_else(missing)?;
        let error = child.stderr.take().ok_or_else(missing)?;
        nonblocking(&input)?;
        nonblocking(&output)?;
        nonblocking(&error)?;
        Ok(Self {
            input: Some(input),
            output: Some(output),
            error: Some(error),
        })
    }

    fn ready(&self, deadline: Instant) -> Result<Vec<PipeKind>, Error> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|time| !time.is_zero())
            .ok_or_else(timeout_error)?;
        let timeout = Timespec {
            tv_sec: remaining.as_secs() as i64,
            tv_nsec: i64::from(remaining.subsec_nanos()),
        };
        let mut kinds = Vec::new();
        let mut descriptors = Vec::new();
        if let Some(input) = &self.input {
            kinds.push(PipeKind::Input);
            descriptors.push(PollFd::new(input, PollFlags::OUT));
        }
        if let Some(output) = &self.output {
            kinds.push(PipeKind::Output);
            descriptors.push(PollFd::new(output, PollFlags::IN));
        }
        if let Some(error) = &self.error {
            kinds.push(PipeKind::Error);
            descriptors.push(PollFd::new(error, PollFlags::IN));
        }
        match poll(&mut descriptors, Some(&timeout)) {
            Ok(_) => Ok(kinds
                .into_iter()
                .zip(descriptors)
                .filter_map(|(kind, fd)| (!fd.revents().is_empty()).then_some(kind))
                .collect()),
            Err(rustix::io::Errno::INTR) => Ok(Vec::new()),
            Err(error) => Err(transport_error(error)),
        }
    }
}

fn communicate(
    child: &mut Child,
    frame: &[u8],
    deadline: Instant,
    max_output: usize,
) -> Result<Vec<u8>, Error> {
    let mut pipes = Pipes::take(child)?;
    let mut sent = 0;
    let mut received = 0;
    let mut output = Vec::new();
    let mut chunk = vec![0; CHUNK_SIZE];
    while pipes.input.is_some() || pipes.output.is_some() || pipes.error.is_some() {
        for kind in pipes.ready(deadline)? {
            match kind {
                PipeKind::Input => {
                    if let Some(input) = &mut pipes.input {
                        // sent is advanced only by the count returned by write.
                        let remaining = frame
                            .get(sent..)
                            .ok_or_else(|| transport_error("Invalid worker input offset."))?;
                        let part = remaining
                            .get(..remaining.len().min(CHUNK_SIZE))
                            .ok_or_else(|| transport_error("Invalid worker input chunk."))?;
                        match input.write(part) {
                            Ok(0) => pipes.input = None,
                            Ok(count) => {
                                sent += count;
                                if sent == frame.len() {
                                    pipes.input = None;
                                }
                            }
                            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
                                pipes.input = None
                            }
                            Err(error) if retry_io(&error) => {}
                            Err(error) => return Err(transport_error(error)),
                        }
                    }
                }
                PipeKind::Output => {
                    if let Some(pipe) = &mut pipes.output {
                        if read_chunk(
                            pipe,
                            &mut chunk,
                            &mut received,
                            max_output,
                            Some(&mut output),
                        )? {
                            pipes.output = None;
                        }
                    }
                }
                PipeKind::Error => {
                    if let Some(pipe) = &mut pipes.error {
                        if read_chunk(pipe, &mut chunk, &mut received, max_output, None)? {
                            pipes.error = None;
                        }
                    }
                }
            }
        }
    }
    Ok(output)
}

fn retry_io(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
}

// Returns true at EOF. stderr is counted but not retained or returned.
fn read_chunk(
    pipe: &mut impl Read,
    chunk: &mut [u8],
    received: &mut usize,
    max_output: usize,
    output: Option<&mut Vec<u8>>,
) -> Result<bool, Error> {
    match pipe.read(chunk) {
        Ok(0) => Ok(true),
        Ok(count) => {
            *received = received
                .checked_add(count)
                .filter(|count| *count <= max_output)
                .ok_or_else(|| Error::new("INDEPENDENT_LIMIT", "rhwp output exceeds 64 MiB."))?;
            if let Some(output) = output {
                let bytes = chunk
                    .get(..count)
                    .ok_or_else(|| transport_error("Invalid worker output chunk."))?;
                output.extend_from_slice(bytes);
            }
            Ok(false)
        }
        Err(error) if retry_io(&error) => Ok(false),
        Err(error) => Err(transport_error(error)),
    }
}

fn wait_until(child: &mut Child, deadline: Instant) -> Result<ExitStatus, Error> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|time| !time.is_zero())
            .ok_or_else(timeout_error)?;
        if let Some(status) = child.try_wait().map_err(transport_error)? {
            return Ok(status);
        }
        // No pipes remain; bound process exit with a short deadline-aware wait.
        std::thread::sleep(remaining.min(Duration::from_millis(5)));
    }
}

fn decode_response(output: &[u8], status: ExitStatus) -> Result<Value, Error> {
    let result: Value = serde_json::from_slice(output)
        .map_err(|_| Error::new("INDEPENDENT_PROTOCOL", "rhwp returned invalid JSON."))?;
    let object = result.as_object().ok_or_else(|| {
        Error::new(
            "INDEPENDENT_PROTOCOL",
            "rhwp returned a non-object response.",
        )
    })?;
    if let Some(error) = object.get("error") {
        let error = error
            .as_object()
            .ok_or_else(|| Error::new("INDEPENDENT_PROTOCOL", "Invalid rhwp error response."))?;
        let code = error
            .get("code")
            .map(protocol_string)
            .unwrap_or_else(|| "RHWP_FAILED".into());
        let message = error
            .get("message")
            .map(protocol_string)
            .unwrap_or_else(|| "rhwp failed.".into());
        return Err(Error::new(code, message));
    }
    if !status.success() {
        return Err(Error::new(
            "INDEPENDENT_FAILED",
            "rhwp exited without a valid result.",
        ));
    }
    Ok(result)
}

fn protocol_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        _ => value.to_string(),
    }
}
