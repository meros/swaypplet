//! greetd IPC worker thread.
//!
//! Owns the `$GREETD_SOCK` connection and speaks the strictly synchronous
//! greetd protocol: one request on the wire, then block until its reply.
//! The reply to `post_auth_message_response` only arrives when PAM produces
//! its next event — acking pam_fprintd's "place your finger" message parks
//! this thread for the whole fingerprint wait, and nothing (not even a
//! cancel) reaches greetd until it resolves. The controller works around
//! that by delaying the ack (see `super`).
//!
//! Requests come in over a std mpsc channel; every reply is translated to
//! an [`Ev`] the GTK side drains from a periodic timeout.

use std::os::unix::net::UnixStream;
use std::sync::mpsc;

use greetd_ipc::codec::SyncCodec;
use greetd_ipc::{AuthMessageType, ErrorType, Request, Response};

pub enum Req {
    Create { username: String },
    Respond(Option<String>),
    Cancel,
    Start { cmd: String },
}

pub enum Ev {
    /// PAM conversation message (reply to Create/Respond).
    AuthMessage { kind: AuthMessageType, text: String },
    /// Conversation complete — the session may be started.
    SessionReady,
    /// start_session succeeded; the greeter must exit so greetd hands over.
    SessionStarted,
    /// cancel_session acknowledged. An error reply counts too: cancel racing
    /// an already-dead session lands in the same "no session" state.
    Canceled,
    /// greetd rejected the request; the session (if any) is gone.
    Error { auth: bool, text: String },
    /// Socket-level failure — the worker has exited.
    Fatal(String),
}

pub fn start() -> (mpsc::Sender<Req>, mpsc::Receiver<Ev>) {
    let (req_tx, req_rx) = mpsc::channel::<Req>();
    let (ev_tx, ev_rx) = mpsc::channel::<Ev>();
    std::thread::spawn(move || {
        let mut stream = match std::env::var("GREETD_SOCK")
            .map_err(|e| e.to_string())
            .and_then(|path| UnixStream::connect(path).map_err(|e| e.to_string()))
        {
            Ok(stream) => stream,
            Err(e) => {
                let _ = ev_tx.send(Ev::Fatal(format!("greetd socket: {e}")));
                return;
            }
        };
        for req in req_rx {
            let (request, is_cancel, is_start) = match req {
                Req::Create { username } => (Request::CreateSession { username }, false, false),
                Req::Respond(response) => {
                    (Request::PostAuthMessageResponse { response }, false, false)
                }
                Req::Cancel => (Request::CancelSession, true, false),
                Req::Start { cmd } => (
                    Request::StartSession {
                        cmd: vec![cmd],
                        env: vec![],
                    },
                    false,
                    true,
                ),
            };
            let reply = request
                .write_to(&mut stream)
                .and_then(|()| Response::read_from(&mut stream));
            let ev = match reply {
                Ok(Response::Success) if is_cancel => Ev::Canceled,
                Ok(Response::Success) if is_start => Ev::SessionStarted,
                Ok(Response::Success) => Ev::SessionReady,
                Ok(Response::AuthMessage {
                    auth_message_type,
                    auth_message,
                }) => Ev::AuthMessage {
                    kind: auth_message_type,
                    text: auth_message,
                },
                Ok(Response::Error { .. }) if is_cancel => Ev::Canceled,
                Ok(Response::Error {
                    error_type,
                    description,
                }) => Ev::Error {
                    auth: matches!(error_type, ErrorType::AuthError),
                    text: description,
                },
                Err(e) => Ev::Fatal(format!("greetd ipc: {e}")),
            };
            let done = matches!(ev, Ev::Fatal(_) | Ev::SessionStarted);
            if ev_tx.send(ev).is_err() || done {
                return;
            }
        }
    });
    (req_tx, ev_rx)
}
