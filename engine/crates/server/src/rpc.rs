//! The wire between the engine and its clients: JSON-RPC 2.0, one message to a line.
//!
//! What a client sees is written down in `PROTOCOL.md` at the root of the workspace; this
//! module reads requests from lines and writes responses and notifications as lines.

use std::sync::Arc;

use columbina_model::TransactionError;
use serde::de::{Deserialize, Deserializer};
use serde::Serialize;
use serde_json::error::Category;
use serde_json::value::RawValue;

/// The version of the protocol that this engine speaks.
pub const PROTOCOL: u32 = 1;

/// The longest line that the engine reads, without its newline.
pub const MAX_LINE: usize = 16 * 1024 * 1024;

/// The longest id of a request, as it is written, quotes and all.
pub const LONGEST_ID: usize = 256;

/// A message as it is sent: a JSON text and a newline. The same line is shared by every
/// connection that it is sent to.
pub type Line = Arc<[u8]>;

/// The line is not JSON.
pub const PARSE_ERROR: i64 = -32700;
/// The message is not a request, or not one that may be sent now.
pub const INVALID_REQUEST: i64 = -32600;
/// There is no such method.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// The params of the method are not what it takes.
pub const INVALID_PARAMS: i64 = -32602;
/// A transaction, an undo or a redo was refused, and the project is as it was.
pub const REFUSED: i64 = -32001;
/// A file that was to be taken in cannot be read, and the project is as it was.
pub const CANNOT_READ: i64 = -32002;
/// The view cannot be made, or there is none, or its window is gone.
pub const VIEW_FAILED: i64 = -32003;

/// An error that a request is answered with.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Fault {
    /// One of the codes of this module.
    pub code: i64,
    /// What went wrong, for a person to read.
    pub message: String,
    /// What went wrong, for a program to read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl Fault {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    #[must_use]
    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }

    /// The params of a method are not what it takes.
    pub fn params(message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, message)
    }

    /// A transaction, an undo or a redo was refused: the error of the transaction is the data.
    pub fn refused(error: &TransactionError) -> Self {
        let data = serde_json::to_value(error).unwrap_or(serde_json::Value::Null);
        Self::new(REFUSED, error.to_string()).with_data(data)
    }
}

/// A request, as it was read from a line.
#[derive(Debug)]
pub struct Request<'a> {
    /// Its id, as it was written: a string, a number or null. A message without one is a
    /// notification, which is not answered.
    pub id: Option<&'a RawValue>,
    /// The method that it calls.
    pub method: String,
    /// Its params, as they were written. Left out and `null` are both none.
    pub params: Option<&'a RawValue>,
}

/// A request as it is written, before it is checked.
#[derive(serde::Deserialize)]
struct Envelope<'a> {
    #[serde(default)]
    jsonrpc: Option<String>,
    #[serde(borrow, default, deserialize_with = "present")]
    id: Option<&'a RawValue>,
    #[serde(default)]
    method: Option<String>,
    #[serde(borrow, default)]
    params: Option<&'a RawValue>,
}

/// An id that is written, even as `null`, as some: so that a request with an id of `null` is
/// told from a notification, which has no id.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<&'de RawValue>, D::Error> {
    <&RawValue>::deserialize(deserializer).map(Some)
}

/// Read a request from a line.
///
/// # Errors
/// The line is not JSON, or not a request: the error to answer with, and the id to answer
/// it to, where the request has an id that could be read.
pub fn parse(line: &[u8]) -> Result<Request<'_>, (Option<&RawValue>, Fault)> {
    let envelope: Envelope = serde_json::from_slice(line).map_err(|error| {
        let fault = match error.classify() {
            Category::Data => Fault::new(
                INVALID_REQUEST,
                format!("the message is not a request object: {error}"),
            ),
            _ => Fault::new(PARSE_ERROR, format!("the line is not JSON: {error}")),
        };
        (None, fault)
    })?;
    if let Some(id) = envelope.id {
        if !is_id(id) {
            let fault = Fault::new(
                INVALID_REQUEST,
                format!(
                    "an id is a string or a whole number, written in at most {LONGEST_ID} bytes"
                ),
            );
            return Err((None, fault));
        }
    }
    let id = envelope.id;
    if envelope.jsonrpc.as_deref() != Some("2.0") {
        return Err((id, Fault::new(INVALID_REQUEST, r#"jsonrpc must be "2.0""#)));
    }
    let Some(method) = envelope.method else {
        return Err((
            id,
            Fault::new(INVALID_REQUEST, "the request names no method"),
        ));
    };
    Ok(Request {
        id,
        method,
        params: envelope.params,
    })
}

/// Whether a value may be the id of a request: a string or a whole number, written in at most
/// [`LONGEST_ID`] bytes. Every answer carries its id back, so a long one would be queued again
/// and again.
fn is_id(id: &RawValue) -> bool {
    let text = id.get().as_bytes();
    let whole = |digits: &[u8]| !digits.is_empty() && digits.iter().all(u8::is_ascii_digit);
    text.len() <= LONGEST_ID
        && match text {
            [b'"', ..] => true,
            [b'-', digits @ ..] | digits => whole(digits),
        }
}

/// Read the params of a method that takes none: they may be left out, or be `null` or `{}`.
///
/// # Errors
/// There are params.
pub fn no_params(method: &str, params: Option<&RawValue>) -> Result<(), Fault> {
    let empty = params.is_none_or(|params| {
        serde_json::from_str::<serde_json::Value>(params.get())
            .is_ok_and(|value| value.as_object().is_some_and(serde_json::Map::is_empty))
    });
    if empty {
        Ok(())
    } else {
        Err(Fault::params(format!("{method} takes no params")))
    }
}

/// Read the params of a method into `T`.
///
/// # Errors
/// There are none, they are not an object, or they are not a `T`; the error names the method and
/// what it takes.
pub fn params<'a, T: Deserialize<'a>>(
    method: &str,
    takes: &str,
    params: Option<&'a RawValue>,
) -> Result<T, Fault> {
    let params = params
        .filter(|params| params.get().starts_with('{'))
        .ok_or_else(|| Fault::params(format!("{method} takes an object, {takes}")))?;
    serde_json::from_str(params.get())
        .map_err(|error| Fault::params(format!("the params of {method}, {takes}: {error}")))
}

#[derive(Serialize)]
struct Response<'a, T: ?Sized> {
    jsonrpc: &'static str,
    id: &'a RawValue,
    result: &'a T,
}

#[derive(Serialize)]
struct Failure<'a> {
    jsonrpc: &'static str,
    id: Option<&'a RawValue>,
    error: &'a Fault,
}

#[derive(Serialize)]
struct Notification<'a, T: ?Sized> {
    jsonrpc: &'static str,
    method: &'a str,
    params: &'a T,
}

/// The line that answers the request `id` with `result`.
pub fn result_line<T: Serialize + ?Sized>(id: &RawValue, result: &T) -> Line {
    to_line(&Response {
        jsonrpc: "2.0",
        id,
        result,
    })
}

/// The line that answers the request `id`, or a request whose id could not be read, with an
/// error.
pub fn error_line(id: Option<&RawValue>, error: &Fault) -> Line {
    to_line(&Failure {
        jsonrpc: "2.0",
        id,
        error,
    })
}

/// The line of a notification.
pub fn notification_line<T: Serialize + ?Sized>(method: &str, params: &T) -> Line {
    to_line(&Notification {
        jsonrpc: "2.0",
        method,
        params,
    })
}

fn to_line(message: &impl Serialize) -> Line {
    // Only maps with keys that are not strings fail to be written, and the engine writes none.
    let mut bytes = serde_json::to_vec(message).expect("a message of the engine is written");
    bytes.push(b'\n');
    bytes.into()
}

/// What is read from a connection: a line, or a line that was too long and was let go.
#[derive(Debug, PartialEq, Eq)]
pub enum Frame {
    /// A line, without its newline.
    Line(Vec<u8>),
    /// A line longer than the most that is read. What came of it was let go, and so is the
    /// rest of it, up to its newline.
    TooLong,
}

/// What cuts the bytes of a connection into lines. A line longer than `most` is never held
/// whole: once it is that long, it is let go, and what comes up to its newline is skipped.
#[derive(Debug)]
pub struct Lines {
    /// The start of a line whose newline has not come yet.
    partial: Vec<u8>,
    /// Whether the rest of a line that was too long is being skipped.
    skipping: bool,
    most: usize,
}

impl Lines {
    pub fn new(most: usize) -> Self {
        Self {
            partial: Vec::new(),
            skipping: false,
            most,
        }
    }

    /// Take in the bytes that were read, and add the frames that they finish to `out`.
    pub fn feed(&mut self, mut data: &[u8], out: &mut Vec<Frame>) {
        while let Some(at) = memchr::memchr(b'\n', data) {
            let head = &data[..at];
            data = &data[at + 1..];
            if self.skipping {
                // The end of a line that was reported already.
                self.skipping = false;
            } else if self.partial.len() + head.len() > self.most {
                self.partial = Vec::new();
                out.push(Frame::TooLong);
            } else {
                let mut line = std::mem::take(&mut self.partial);
                line.extend_from_slice(head);
                out.push(Frame::Line(line));
            }
        }
        if self.skipping || data.is_empty() {
            return;
        }
        if self.partial.len() + data.len() > self.most {
            self.partial = Vec::new();
            self.skipping = true;
            out.push(Frame::TooLong);
        } else {
            self.partial.extend_from_slice(data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(lines: &mut Lines, data: &[u8]) -> Vec<Frame> {
        let mut out = Vec::new();
        lines.feed(data, &mut out);
        out
    }

    fn line(text: &str) -> Frame {
        Frame::Line(text.as_bytes().to_vec())
    }

    #[test]
    fn lines_are_cut_at_newlines_across_reads() {
        let mut lines = Lines::new(100);
        assert_eq!(feed(&mut lines, b"ab\ncd"), [line("ab")]);
        assert_eq!(feed(&mut lines, b"ef"), []);
        assert_eq!(
            feed(&mut lines, b"\n\nx\n"),
            [line("cdef"), line(""), line("x")]
        );
    }

    #[test]
    fn a_line_too_long_is_reported_once_and_skipped_to_its_newline() {
        let mut lines = Lines::new(4);
        assert_eq!(feed(&mut lines, b"abc"), []);
        assert_eq!(feed(&mut lines, b"de"), [Frame::TooLong]);
        assert_eq!(feed(&mut lines, b"fgh"), []);
        assert_eq!(feed(&mut lines, b"ij\nok\n"), [line("ok")]);
        // A long line that ends in the same read is reported too, and a line of the most
        // that is read is read.
        assert_eq!(
            feed(&mut lines, b"abcdefg\nabcd\n"),
            [Frame::TooLong, line("abcd")]
        );
        assert_eq!(feed(&mut lines, b"ab"), []);
        assert_eq!(feed(&mut lines, b"cde\n"), [Frame::TooLong]);
        assert_eq!(feed(&mut lines, b"x\n"), [line("x")]);
    }

    fn fault(line: &str) -> (Option<String>, i64) {
        let (id, fault) = parse(line.as_bytes()).unwrap_err();
        (id.map(|id| id.get().to_owned()), fault.code)
    }

    #[test]
    fn a_request_is_read_with_its_id_as_it_was_written() {
        let request =
            parse(br#"{"jsonrpc":"2.0","id":"a\"b","method":"hello","params":{"x":1}}"#).unwrap();
        assert_eq!(request.id.unwrap().get(), r#""a\"b""#);
        assert_eq!(request.method, "hello");
        assert_eq!(request.params.unwrap().get(), r#"{"x":1}"#);
        let request = parse(br#"{"jsonrpc":"2.0","id":-12,"method":"m"}"#).unwrap();
        assert_eq!(request.id.unwrap().get(), "-12");
        assert!(request.params.is_none());
        let long = format!(
            r#"{{"jsonrpc":"2.0","id":"{}","method":"m"}}"#,
            "a".repeat(254)
        );
        assert_eq!(parse(long.as_bytes()).unwrap().id.unwrap().get().len(), 256);
        let notification = parse(br#"{"jsonrpc":"2.0","method":"m","params":null}"#).unwrap();
        assert!(notification.id.is_none());
        assert!(notification.params.is_none());
    }

    #[test]
    fn what_is_not_a_request_is_refused_with_the_id_where_it_can_be_read() {
        assert_eq!(fault("{"), (None, PARSE_ERROR));
        assert_eq!(fault("nonsense"), (None, PARSE_ERROR));
        assert_eq!(fault("[1]"), (None, INVALID_REQUEST));
        assert_eq!(fault(r#"{"jsonrpc":2,"id":1}"#), (None, INVALID_REQUEST));
        // An id that is not a string or a whole number, or is too long, is not read.
        let long = format!(r#""{}""#, "a".repeat(255));
        for id in [
            "{}",
            "[1]",
            "null",
            "true",
            "1.5",
            "1e3",
            "-0.5",
            long.as_str(),
        ] {
            let line = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"m"}}"#);
            assert_eq!(fault(&line), (None, INVALID_REQUEST), "{id}");
        }
        assert_eq!(
            fault(r#"{"jsonrpc":"1.0","id":7,"method":"m"}"#),
            (Some("7".into()), INVALID_REQUEST)
        );
        assert_eq!(
            fault(r#"{"id":7,"method":"m"}"#),
            (Some("7".into()), INVALID_REQUEST)
        );
        assert_eq!(
            fault(r#"{"jsonrpc":"2.0","id":7}"#),
            (Some("7".into()), INVALID_REQUEST)
        );
    }

    #[test]
    fn empty_params_are_no_params() {
        for params in [None, Some("{}"), Some("{ }")] {
            let raw = params.map(|p| RawValue::from_string(p.into()).unwrap());
            assert_eq!(no_params("m", raw.as_deref()), Ok(()), "{params:?}");
        }
        for params in [r#"{"a":1}"#, "[]", "[1]", "3"] {
            let raw = RawValue::from_string(params.into()).unwrap();
            assert_eq!(no_params("m", Some(&raw)).unwrap_err().code, INVALID_PARAMS);
        }
        // Params are an object, never a list.
        let list = RawValue::from_string(r#"["agent", 1]"#.into()).unwrap();
        let read = params::<(String, u32)>("m", "{}", Some(&list));
        assert_eq!(read.unwrap_err().code, INVALID_PARAMS);
    }

    #[test]
    fn lines_are_written_as_json_rpc() {
        let id = RawValue::from_string("3".into()).unwrap();
        let text = |line: Line| String::from_utf8(line.to_vec()).unwrap();
        assert_eq!(
            text(result_line(&id, &serde_json::json!({"a": 1}))),
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"a\":1}}\n"
        );
        assert_eq!(
            text(error_line(None, &Fault::new(PARSE_ERROR, "no"))),
            "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32700,\"message\":\"no\"}}\n"
        );
        let refused = Fault::refused(&TransactionError::Empty);
        assert_eq!(
            text(error_line(Some(&id), &refused)),
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"error\":{\"code\":-32001,\"message\":\"the \
             transaction has no operations\",\"data\":{\"message\":\"the transaction has no \
             operations\",\"reason\":\"empty\"}}}\n"
        );
        assert_eq!(
            text(notification_line("m", &[1])),
            "{\"jsonrpc\":\"2.0\",\"method\":\"m\",\"params\":[1]}\n"
        );
    }
}
