//! The agent interface (D-018): a local HTTP API inside Burrow, and the `burrow --mcp` /
//! `burrow --agent` clients that talk to it.
//!
//! The server listens on 127.0.0.1 at a random port and writes `{port, token}` to
//! `<data>/agent.json`. Every request needs `Authorization: Bearer <token>`; requests that carry
//! an `Origin` header (i.e. come from a web page) are refused. Tools run on the UI thread
//! (`browser::Burrow::agent_call`); the connection's thread waits for the [`Reply`].

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

use burrow_core::agent::tools_json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{browser, paths, task};

/// How long a tool may take before the caller gets an error.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_BODY: usize = 8 << 20;

#[derive(Serialize, Deserialize)]
struct Endpoint {
    port: u16,
    token: String,
    pid: u32,
}

fn endpoint_file() -> PathBuf {
    paths::data_dir().join("agent.json")
}

/// The answer to one tool call. Dropping it unanswered tells the caller the call failed.
pub struct Reply(mpsc::Sender<Result<Value, String>>);

impl Reply {
    pub fn send(self, result: Result<Value, String>) {
        let _ = self.0.send(result);
    }
}

// ---- Server (inside Burrow) ----------------------------------------------------------------------

/// Starts the API server on its own thread and publishes its endpoint.
pub fn start_server() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(e) => return eprintln!("Burrow: agent API unavailable: {e}"),
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let token = format!("{}{}", burrow_core::model::new_id(), burrow_core::model::new_id()).replace('-', "");
    let endpoint = Endpoint { port, token: token.clone(), pid: std::process::id() };
    if let Err(e) = write_private(&endpoint_file(), &serde_json::to_vec_pretty(&endpoint).unwrap_or_default()) {
        eprintln!("Burrow: could not write {}: {e}", endpoint_file().display());
    }
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let token = token.clone();
            std::thread::spawn(move || serve(stream, &token, port));
        }
    });
}

/// Writes a file only the user can read (the token in it grants full control of Burrow).
/// On Windows, the data folder in %LOCALAPPDATA% is already private to the user.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let _ = std::fs::remove_file(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(bytes)
}

/// Forgets the endpoint at exit, so clients know Burrow isn't running.
pub fn stop_server() {
    let file = endpoint_file();
    let ours = std::fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Endpoint>(&b).ok())
        .is_some_and(|e| e.pid == std::process::id());
    if ours {
        let _ = std::fs::remove_file(file);
    }
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or_default().to_string(), parts.next().unwrap_or_default().to_string());
    let mut headers = vec![];
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).map_err(|e| e.to_string())? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
        if headers.len() > 100 {
            return Err("too many headers".into());
        }
    }
    let mut req = Request { method, path, headers, body: vec![] };
    let len: usize = req.header("content-length").and_then(|l| l.parse().ok()).unwrap_or(0);
    if len > MAX_BODY {
        return Err("body too large".into());
    }
    req.body = vec![0; len];
    reader.read_exact(&mut req.body).map_err(|e| e.to_string())?;
    Ok(req)
}

fn respond(stream: &mut TcpStream, status: u16, body: &Value) {
    let body = body.to_string();
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(body.as_bytes()));
}

fn serve(mut stream: TcpStream, token: &str, port: u16) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let req = match read_request(&mut stream) {
        Ok(r) => r,
        Err(e) => return respond(&mut stream, 400, &json!({ "ok": false, "error": e })),
    };
    // Web pages can reach localhost too: refuse anything a browser sent (it always adds Origin
    // to POSTs) and any Host that isn't ours (DNS rebinding).
    let host_ok = req.header("host").is_some_and(|h| h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}"));
    if req.header("origin").is_some() || !host_ok {
        return respond(&mut stream, 403, &json!({ "ok": false, "error": "forbidden" }));
    }
    if req.header("authorization") != Some(&format!("Bearer {token}")) {
        return respond(&mut stream, 401, &json!({ "ok": false, "error": "missing or wrong token (see agent.json)" }));
    }
    let route = req.path.trim_start_matches('/').strip_prefix("v1/").unwrap_or("").to_string();
    match (req.method.as_str(), route.as_str()) {
        ("GET", "tools") => respond(&mut stream, 200, &json!({ "ok": true, "result": tools_json() })),
        ("POST", tool) if burrow_core::agent::tool(tool).is_some() => {
            let args: Value = if req.body.is_empty() {
                json!({})
            } else {
                match serde_json::from_slice(&req.body) {
                    Ok(v) => v,
                    Err(e) => return respond(&mut stream, 400, &json!({ "ok": false, "error": format!("bad JSON: {e}") })),
                }
            };
            match call_on_ui(tool.to_string(), args) {
                Ok(result) => respond(&mut stream, 200, &json!({ "ok": true, "result": result })),
                Err(error) => respond(&mut stream, 200, &json!({ "ok": false, "error": error })),
            }
        }
        _ => respond(&mut stream, 404, &json!({ "ok": false, "error": "unknown route; use GET /v1/tools or POST /v1/<tool>" })),
    }
}

fn call_on_ui(tool: String, args: Value) -> Result<Value, String> {
    let (tx, rx) = mpsc::channel();
    let reply = Reply(tx);
    task::post_ui(move || {
        browser::with(|b| b.agent_call(&tool, args, reply));
    });
    match rx.recv_timeout(CALL_TIMEOUT) {
        Ok(r) => r,
        Err(mpsc::RecvTimeoutError::Timeout) => Err("timed out".into()),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err("Burrow is busy or shutting down".into()),
    }
}

// ---- Client (`burrow --agent`, `burrow --mcp`) -------------------------------------------------------

fn read_endpoint() -> Option<Endpoint> {
    serde_json::from_slice(&std::fs::read(endpoint_file()).ok()?).ok()
}

fn http(endpoint: &Endpoint, method: &str, path: &str, body: &Value) -> std::io::Result<Value> {
    let mut stream = TcpStream::connect(("127.0.0.1", endpoint.port))?;
    stream.set_read_timeout(Some(CALL_TIMEOUT + Duration::from_secs(5)))?;
    let body = body.to_string();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        endpoint.port,
        endpoint.token,
        body.len()
    )?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let split = response.windows(4).position(|w| w == b"\r\n\r\n").map_or(response.len(), |i| i + 4);
    serde_json::from_slice(&response[split..]).map_err(std::io::Error::other)
}

/// Starts Burrow in the background (when an agent calls and it isn't running).
fn launch_burrow() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start Burrow: {e}"))
}

/// Calls a tool in the running Burrow, starting Burrow first if needed.
pub fn call(tool: &str, args: &Value) -> Result<Value, String> {
    let (method, path, body) = match tool {
        "tools" => ("GET", "/v1/tools".to_string(), Value::Null),
        _ => ("POST", format!("/v1/{tool}"), args.clone()),
    };
    let mut launched = false;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(endpoint) = read_endpoint() {
            match http(&endpoint, method, &path, &body) {
                Ok(v) if v["ok"] == true => return Ok(v["result"].clone()),
                Ok(v) => return Err(v["error"].as_str().unwrap_or("unknown error").to_string()),
                // A stale agent.json from a Burrow that crashed: start a fresh one.
                Err(e) if e.kind() != std::io::ErrorKind::ConnectionRefused => return Err(e.to_string()),
                Err(_) => {}
            }
        }
        if !launched {
            launch_burrow()?;
            launched = true;
        }
        if Instant::now() > deadline {
            return Err("Burrow didn't start in time".into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// `--data-dir <path>` points the client at another Burrow data folder (e.g. a test profile).
fn take_data_dir(args: &mut Vec<String>) {
    if let Some(i) = args.iter().position(|a| a == "--data-dir")
        && i + 1 < args.len()
    {
        let dir = args.remove(i + 1);
        args.remove(i);
        // Safe: runs first thing in main, before any other thread exists.
        unsafe { std::env::set_var("BURROW_DATA_DIR", dir) };
    }
}

/// Handles `burrow --mcp` and `burrow --agent …`. Returns the exit code, or `None` to start the
/// browser normally.
pub fn main_if_requested() -> Option<i32> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--mcp") => {
            take_data_dir(&mut args);
            Some(run_mcp())
        }
        Some("--agent") => {
            take_data_dir(&mut args);
            Some(run_cli(&args[1..]))
        }
        _ => None,
    }
}

const CLI_HELP: &str = "usage: burrow --agent <tool> [json-args] [--out file.png] [--data-dir dir]
       burrow --agent tools          list the tools and their arguments
       burrow --mcp [--data-dir dir] run as an MCP server on stdin/stdout
example: burrow --agent open_tab '{\"url\":\"example.com\"}'";

/// Release builds on Windows are GUI programs with no console; borrow the terminal's console so
/// `burrow --agent` can print (unless output is already redirected, e.g. to a pipe).
#[cfg(windows)]
fn attach_console() {
    unsafe extern "system" {
        fn GetStdHandle(which: u32) -> isize;
        fn AttachConsole(process: u32) -> i32;
    }
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    unsafe {
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        if out == 0 || out == -1 {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn run_cli(args: &[String]) -> i32 {
    #[cfg(windows)]
    attach_console();
    let mut args = args.to_vec();
    let out = args.iter().position(|a| a == "--out").map(|i| {
        args.remove(i);
        if i < args.len() { args.remove(i) } else { String::new() }
    });
    let Some(tool) = args.first() else {
        eprintln!("{CLI_HELP}");
        return 2;
    };
    let params: Value = match args.get(1) {
        Some(j) => match serde_json::from_str(j) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("bad JSON arguments: {e}");
                return 2;
            }
        },
        None => json!({}),
    };
    match call(tool, &params) {
        Ok(result) => {
            if let (Some(path), Some(data)) = (&out, result["data"].as_str().filter(|_| result["type"] == "image")) {
                match base64_decode(data).map(|png| std::fs::write(path, png)) {
                    Some(Ok(())) => println!("saved {path}"),
                    _ => {
                        eprintln!("could not save {path}");
                        return 1;
                    }
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&result).unwrap_or_default());
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        acc = (acc << 6) | value(c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

// ---- MCP server over stdio ---------------------------------------------------------------------

const MCP_INSTRUCTIONS: &str = "Burrow is an Arc-style browser with one window. Its sidebar has spaces; each space has \
favorites (app tiles), pinned tabs in folders (each has a fixed place and a home URL), and today's tabs. Spaces usually \
have separate logins. Start with list_spaces or list_tabs to get ids. open_tab opens in the background by default so \
you don't disturb the user; pass focus=true to show a tab. read_page, screenshot, click, type_text, press_key, \
wait_for and eval_js work on any tab, shown or not. Prefer read_page format=elements to find what to click.";

fn run_mcp() -> i32 {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } });
                let _ = writeln!(stdout, "{err}").and_then(|_| stdout.flush());
                continue;
            }
        };
        // Notifications (no id) need no answer.
        let Some(id) = msg.get("id").cloned() else { continue };
        let response = match mcp_handle(msg["method"].as_str().unwrap_or_default(), &msg["params"]) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
        };
        if writeln!(stdout, "{response}").and_then(|_| stdout.flush()).is_err() {
            break;
        }
    }
    0
}

fn mcp_handle(method: &str, params: &Value) -> Result<Value, (i32, String)> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "burrow", "version": env!("CARGO_PKG_VERSION") },
            "instructions": MCP_INSTRUCTIONS,
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools_json() })),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or_default();
            if burrow_core::agent::tool(name).is_none() {
                return Err((-32602, format!("unknown tool {name}")));
            }
            let args = if params["arguments"].is_object() { params["arguments"].clone() } else { json!({}) };
            Ok(match call(name, &args) {
                Ok(r) if r["type"] == "image" => json!({ "content": [{ "type": "image", "data": r["data"], "mimeType": r["mimeType"] }] }),
                Ok(r) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&r).unwrap_or_default() }] }),
                Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
            })
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("iVBORw0K").unwrap(), [0x89, b'P', b'N', b'G', b'\r', b'\n']);
    }

    #[test]
    fn mcp_lists_tools_and_rejects_unknown_ones() {
        let tools = mcp_handle("tools/list", &json!({})).unwrap();
        assert!(tools["tools"].as_array().unwrap().iter().any(|t| t["name"] == "screenshot"));
        assert!(mcp_handle("tools/call", &json!({ "name": "rm_rf" })).is_err());
        assert!(mcp_handle("nope", &json!({})).is_err());
    }
}
