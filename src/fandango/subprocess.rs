//! Python runs only in a **child** (`Command::current_exe()`), so OOM/SIGKILL there returns an error here instead of killing the fuzzer.
//! A `#[ctor]` at the bottom of this file detects worker `argv` and runs the loop **before `main`**.
//!
//! IPC is **length-prefixed `postcard`** over stdin/stdout (see `read_msg` / `write_msg`); no hand-maintained tag bytes.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use ctor::ctor;
use log::{error, warn};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::fandango::{FandangoClient, FandangoInprocessModule, FandangoModuleInitError};

fn subprocess_init(i: FandangoSubprocessInitIpc) -> FandangoModuleInitError {
    FandangoModuleInitError::SubprocessIpc(i)
}

/// Subprocess-only failures while starting the IPC worker (never returned by [`FandangoInprocessModule`](inprocess::FandangoInprocessModule)).
#[derive(Debug)]
pub enum FandangoSubprocessInitIpc {
    Io(std::io::Error),
    KwargsJson(serde_json::Error),
    MissingPipe(&'static str),
    HandshakeFailed(String),
    /// Python setup failed in the worker (same text as [`FandangoPythonModuleInitError::format_report`] would produce there).
    WorkerSetupFailed(String),
}

impl std::fmt::Display for FandangoSubprocessInitIpc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::KwargsJson(e) => write!(f, "could not serialize kwargs to JSON: {e}"),
            Self::MissingPipe(what) => write!(f, "IPC worker missing {what} pipe"),
            Self::HandshakeFailed(msg) => write!(f, "{msg}"),
            Self::WorkerSetupFailed(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for FandangoSubprocessInitIpc {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::KwargsJson(e) => Some(e),
            Self::MissingPipe(_) | Self::HandshakeFailed(_) | Self::WorkerSetupFailed(_) => None,
        }
    }
}

/// `argv[1]` when this executable is the IPC worker.
pub const IPC_WORKER_ARG: &str = "__libafl_fandango_ipc_worker__";

/// Reject absurd frames (misbehaving peer / corruption).
const MAX_FRAME_BYTES: u32 = 1024 * 1024 * 1024;

/// After closing stdin, wait this long for the worker to exit before `SIGKILL` ([`Drop`] path).
const DROP_GRACEFUL_WAIT: Duration = Duration::from_millis(750);

#[derive(Serialize, Deserialize)]
enum IpcHandshake {
    Ready,
    Failed(String),
}

#[derive(Serialize, Deserialize)]
enum IpcReq {
    Next,
    Parse(Vec<u8>),
}

#[derive(Serialize, Deserialize)]
enum IpcResp {
    NextOk(Vec<u8>),
    ParseOk(u32),
    CallErr(String),
}

fn write_msg<W: Write, T: Serialize>(w: &mut W, msg: &T) -> Result<(), String> {
    let bytes = postcard::to_stdvec(msg).map_err(|e| e.to_string())?;
    let len: u32 = bytes
        .len()
        .try_into()
        .map_err(|_| "IPC message too large".to_string())?;
    w.write_all(&len.to_be_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&bytes).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

/// `Ok(None)` = clean EOF before the next frame (stdin closed).
fn read_msg<R: Read, T: DeserializeOwned>(r: &mut R) -> Result<Option<T>, String> {
    let mut lenb = [0u8; 4];
    match r.read_exact(&mut lenb) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.to_string()),
    }
    let len = u32::from_be_bytes(lenb);
    if len > MAX_FRAME_BYTES {
        return Err(format!("IPC frame too large: {len}"));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).map_err(|e| e.to_string())?;
    postcard::from_bytes(&buf)
        .map_err(|e| e.to_string())
        .map(Some)
}

fn kwargs_json(kwargs: &[(&str, &str)]) -> Result<String, serde_json::Error> {
    serde_json::to_string(&std::collections::HashMap::<_, _>::from_iter(
        kwargs.iter().copied(),
    ))
}

fn pydict_from_kwargs_json<'py>(py: Python<'py>, json: &str) -> Result<Bound<'py, PyDict>, String> {
    let v: JsonValue = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let obj = v
        .as_object()
        .ok_or_else(|| "kwargs JSON must be an object".to_string())?;
    let d = PyDict::new(py);
    for (k, val) in obj {
        let s = val
            .as_str()
            .ok_or_else(|| format!("kwargs[{k}] must be a JSON string"))?;
        d.set_item(k, s).map_err(|e| e.to_string())?;
    }
    Ok(d)
}

pub(crate) fn exit_now_if_ipc_worker_argv() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 5 || a[1] != IPC_WORKER_ARG {
        return;
    }
    if let Err(e) = run_worker(&a[2], &a[3], &a[4]) {
        error!("fandango_ipc worker: {e}");
        eprintln!("fandango-ipc: {e}");
        std::process::exit(1);
    }
    std::process::exit(0);
}

fn run_worker(interface: &str, fan_file: &str, kwargs_json: &str) -> Result<(), String> {
    let mut out = io::stdout().lock();
    let mut inp = io::stdin().lock();

    Python::with_gil(|py| {
        let kwargs = pydict_from_kwargs_json(py, kwargs_json)?;
        let (module, generator) = match FandangoInprocessModule::load_interface_and_setup(
            py, interface, fan_file, &kwargs,
        ) {
            Ok(x) => x,
            Err(e) => {
                let text = e.format_report();
                error!("fandango_ipc worker Python setup failed: {text}");
                write_msg(&mut out, &IpcHandshake::Failed(text.clone()))?;
                return Err(text);
            }
        };

        write_msg(&mut out, &IpcHandshake::Ready)?;

        while let Some(req) = read_msg::<_, IpcReq>(&mut inp)? {
            let resp = match req {
                IpcReq::Next => {
                    let res: PyResult<Vec<u8>> = (|| {
                        let w = generator.clone_ref(py);
                        module
                            .getattr(py, "next_input")?
                            .call1(py, (w,))?
                            .extract::<Vec<u8>>(py)
                    })();
                    match res {
                        Ok(bytes) => IpcResp::NextOk(bytes),
                        Err(e) => IpcResp::CallErr(e.to_string()),
                    }
                }
                IpcReq::Parse(buf) => {
                    let res: PyResult<u32> = (|| {
                        let w = generator.clone_ref(py);
                        module
                            .getattr(py, "parse_input")?
                            .call1(py, (w, buf.as_slice()))?
                            .extract::<u32>(py)
                    })();
                    match res {
                        Ok(n) => IpcResp::ParseOk(n),
                        Err(e) => IpcResp::CallErr(e.to_string()),
                    }
                }
            };
            write_msg(&mut out, &resp)?;
        }
        Ok(())
    })
}

/// A module for running Fandango in a subprocess.
///
/// Trades off some speed (due to IPC) for more robust error handling (e.g. if the child process OOMs).
///
/// Essentially a wrapper around `FandangoInprocessModule` that runs it in a subprocess and communicates via IPC.
pub struct FandangoSubprocessModule {
    child: Option<Child>,
    stdin: Option<io::BufWriter<ChildStdin>>,
    stdout: Option<io::BufReader<ChildStdout>>,
}

impl FandangoSubprocessModule {
    pub fn new(
        fandango_file: &str,
        kwargs: &[(&str, &str)],
    ) -> Result<Self, FandangoModuleInitError> {
        let iface = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/run_fandango.py");
        Self::with_custom_python_interface(
            iface.to_str().ok_or_else(|| {
                FandangoModuleInitError::FilePathError(
                    "default interface path is not UTF-8".to_string(),
                )
            })?,
            fandango_file,
            kwargs,
        )
    }

    pub fn with_custom_python_interface(
        python_interface_path: &str,
        fandango_file: &str,
        kwargs: &[(&str, &str)],
    ) -> Result<Self, FandangoModuleInitError> {
        let exe = std::env::current_exe()
            .map_err(|e| subprocess_init(FandangoSubprocessInitIpc::Io(e)))?;
        let kw = kwargs_json(kwargs)
            .map_err(|e| subprocess_init(FandangoSubprocessInitIpc::KwargsJson(e)))?;

        let mut child = Command::new(&exe)
            .arg(IPC_WORKER_ARG)
            .arg(python_interface_path)
            .arg(fandango_file)
            .arg(&kw)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| subprocess_init(FandangoSubprocessInitIpc::Io(e)))?;

        let pid = child.id();

        let stdin = io::BufWriter::new(
            child
                .stdin
                .take()
                .ok_or_else(|| subprocess_init(FandangoSubprocessInitIpc::MissingPipe("stdin")))?,
        );
        let mut stdout =
            io::BufReader::new(child.stdout.take().ok_or_else(|| {
                subprocess_init(FandangoSubprocessInitIpc::MissingPipe("stdout"))
            })?);

        let hs: IpcHandshake = match read_msg(&mut stdout) {
            Ok(Some(h)) => h,
            Ok(None) => {
                return Err(subprocess_init(FandangoSubprocessInitIpc::HandshakeFailed(
                    ipc_fail(&mut child, "EOF during handshake"),
                )));
            }
            Err(e) => {
                return Err(subprocess_init(FandangoSubprocessInitIpc::HandshakeFailed(
                    ipc_fail(&mut child, e),
                )));
            }
        };
        match hs {
            IpcHandshake::Ready => {}
            IpcHandshake::Failed(msg) => {
                error!("fandango_ipc: worker setup failed (pid={pid}): {msg}");
                let _ = child.wait();
                return Err(subprocess_init(
                    FandangoSubprocessInitIpc::WorkerSetupFailed(msg),
                ));
            }
        }

        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            stdout: Some(stdout),
        })
    }

    /// Close the IPC connection and wait for the worker to exit.
    ///
    /// The worker stops its request loop when stdin closes, then exits normally. Prefer this
    /// over relying on [`Drop`] when you want a clean shutdown without a bounded wait followed by
    /// `SIGKILL`.
    pub fn shutdown(&mut self) -> io::Result<ExitStatus> {
        if self.child.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "Fandango IPC subprocess already shut down",
            ));
        }
        if let Some(s) = self.stdin.as_mut() {
            let _ = s.flush();
        }
        self.stdin.take();
        self.stdout.take();
        self.child.take().expect("child was Some").wait()
    }

    fn rpc(&mut self, req: &IpcReq) -> Result<IpcResp, String> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| "IPC subprocess shut down".to_string())?;
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| "IPC subprocess shut down".to_string())?;
        let stdout = self
            .stdout
            .as_mut()
            .ok_or_else(|| "IPC subprocess shut down".to_string())?;
        write_msg(stdin, req)?;
        match read_msg(stdout).map_err(|e| ipc_fail(child, e))? {
            Some(r) => Ok(r),
            None => Err(ipc_fail(child, "EOF from worker before response")),
        }
    }
}

fn ipc_fail(child: &mut Child, reason: impl std::fmt::Display) -> String {
    let st = child.wait().ok().map(|s| s.to_string()).unwrap_or_default();
    let msg = if st.is_empty() {
        format!("IPC: {reason}")
    } else {
        format!("IPC: {reason} ({st})")
    };
    warn!("{msg}");
    msg
}

impl FandangoClient for FandangoSubprocessModule {
    fn next_input(&mut self) -> Result<Vec<u8>, String> {
        match self.rpc(&IpcReq::Next)? {
            IpcResp::NextOk(b) => Ok(b),
            IpcResp::CallErr(s) => Err(s),
            IpcResp::ParseOk(_) => Err("unexpected ParseOk from worker".into()),
        }
    }

    fn parse_input(&mut self, input: &[u8]) -> Result<u32, String> {
        match self.rpc(&IpcReq::Parse(input.to_vec()))? {
            IpcResp::ParseOk(n) => Ok(n),
            IpcResp::CallErr(s) => Err(s),
            IpcResp::NextOk(_) => Err("unexpected NextOk from worker".into()),
        }
    }
}

impl Drop for FandangoSubprocessModule {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if let Some(s) = self.stdin.as_mut() {
            let _ = s.flush();
        }
        self.stdin.take();
        self.stdout.take();

        let pid = child.id();
        let deadline = Instant::now() + DROP_GRACEFUL_WAIT;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        warn!(
                            "fandango_ipc: worker pid={pid} did not exit after stdin close within {:?}; sending SIGKILL",
                            DROP_GRACEFUL_WAIT
                        );
                        let _ = child.kill();
                        let _ = child.wait();
                        return;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => {
                    warn!("fandango_ipc: try_wait on pid={pid} failed ({e}); sending SIGKILL");
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
            }
        }
    }
}

#[ctor]
fn ipc_worker_ctor() {
    exit_now_if_ipc_worker_argv();
}
