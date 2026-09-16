//! Blocking AWS Lambda Runtime API client (optional `lambda` feature).
//!
//! Uses `ureq` only — MUST NOT pull Tokio. Poll on the host thread or via
//! `call_blocking`; actor futures stay on SOLO/TROUP.

use std::io::Read;
use std::time::Duration;

/// Default Runtime API path prefix.
pub const RUNTIME_API_PREFIX: &str = "/2018-06-01/runtime";

/// One invocation from `GET …/invocation/next`.
#[derive(Debug, Clone)]
// ss[impl graph.lambda.runtime-api]
pub struct LambdaInvocation {
    /// Opaque request id for response/error posts.
    pub request_id: String,
    /// Raw body bytes (typically JSON).
    pub body: Vec<u8>,
}

/// Blocking client for the Lambda Runtime API.
// ss[impl graph.lambda.runtime-api]
// ss[impl platform.lambda-no-tokio]
// ss[impl graph.lambda.host-not-executor]
pub struct LambdaRuntimeApi {
    base: String,
    agent: ureq::Agent,
}

impl LambdaRuntimeApi {
    /// Build from `AWS_LAMBDA_RUNTIME_API` (`host:port`).
    // ss[impl graph.lambda.runtime-api]
    pub fn from_env() -> Result<Self, String> {
        let host = std::env::var("AWS_LAMBDA_RUNTIME_API")
            .map_err(|_| "AWS_LAMBDA_RUNTIME_API not set".to_string())?;
        Self::with_base(format!("http://{host}"))
    }

    /// Build with an explicit base URL (tests inject `http://127.0.0.1:port`).
    // ss[impl graph.lambda.runtime-api]
    pub fn with_base(base: impl Into<String>) -> Result<Self, String> {
        let base = base.into().trim_end_matches('/').to_string();
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(2))
            .timeout_read(Duration::from_secs(900))
            .build();
        Ok(Self { base, agent })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}{}", self.base, RUNTIME_API_PREFIX, path)
    }

    /// Blocking `GET /runtime/invocation/next`.
    // ss[impl graph.lambda.runtime-api]
    pub fn next_invocation(&self) -> Result<LambdaInvocation, String> {
        let resp = self
            .agent
            .get(&self.url("/invocation/next"))
            .call()
            .map_err(|e| format!("GET next: {e}"))?;
        let request_id = resp
            .header("Lambda-Runtime-Aws-Request-Id")
            .ok_or_else(|| "missing Lambda-Runtime-Aws-Request-Id".to_string())?
            .to_string();
        let mut body = Vec::new();
        resp.into_reader()
            .read_to_end(&mut body)
            .map_err(|e| format!("read body: {e}"))?;
        Ok(LambdaInvocation { request_id, body })
    }

    /// Blocking `POST /runtime/invocation/{id}/response`.
    // ss[impl graph.lambda.runtime-api]
    pub fn post_response(&self, request_id: &str, body: &[u8]) -> Result<(), String> {
        let path = format!("/invocation/{request_id}/response");
        self.agent
            .post(&self.url(&path))
            .set("Content-Type", "application/json")
            .send_bytes(body)
            .map_err(|e| format!("POST response: {e}"))?;
        Ok(())
    }

    /// Blocking `POST /runtime/invocation/{id}/error`.
    // ss[impl graph.lambda.runtime-api]
    pub fn post_error(&self, request_id: &str, error_json: &[u8]) -> Result<(), String> {
        let path = format!("/invocation/{request_id}/error");
        self.agent
            .post(&self.url(&path))
            .set("Content-Type", "application/json")
            .send_bytes(error_json)
            .map_err(|e| format!("POST error: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod lambda_runtime_api_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    fn read_http_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("read timeout");
        let mut raw = Vec::new();
        let mut buf = [0u8; 512];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(header_end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = &raw[..header_end + 4];
                        let header_str = String::from_utf8_lossy(headers);
                        let content_len = header_str
                            .lines()
                            .find_map(|l| {
                                let lower = l.to_ascii_lowercase();
                                lower
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        let body_start = header_end + 4;
                        while raw.len() < body_start + content_len {
                            match stream.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n2) => raw.extend_from_slice(&buf[..n2]),
                                Err(_) => break,
                            }
                        }
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&raw).into_owned()
    }

    // ss[verify graph.lambda.runtime-api]
    // ss[verify platform.lambda-no-tokio]
    #[test]
    fn next_and_response_against_local_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let base = format!("http://127.0.0.1:{port}");

        let server = thread::spawn(move || {
            // GET next
            let (mut stream, _) = listener.accept().expect("accept next");
            let _ = read_http_request(&mut stream);
            let resp = b"HTTP/1.1 200 OK\r\n\
Lambda-Runtime-Aws-Request-Id: req-1\r\n\
Content-Length: 2\r\n\
Connection: close\r\n\
\r\n\
42";
            stream.write_all(resp).expect("write next");
            let _ = stream.shutdown(Shutdown::Both);

            // POST response
            let (mut stream, _) = listener.accept().expect("accept response");
            let req = read_http_request(&mut stream);
            assert!(
                req.contains("POST /2018-06-01/runtime/invocation/req-1/response"),
                "unexpected request: {req}"
            );
            assert!(req.contains("84"), "body missing in request: {req}");
            let ok = b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(ok).expect("write ack");
            let _ = stream.shutdown(Shutdown::Both);
        });

        let api = LambdaRuntimeApi::with_base(&base).expect("client");
        let inv = api.next_invocation().expect("next");
        assert_eq!(inv.request_id, "req-1");
        assert_eq!(inv.body, b"42");
        api.post_response(&inv.request_id, b"84").expect("response");
        server.join().expect("server");
    }

    // ss[verify graph.lambda.runtime-api]
    #[test]
    fn post_error_against_local_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let base = format!("http://127.0.0.1:{port}");

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept error");
            let req = read_http_request(&mut stream);
            assert!(
                req.contains("POST /2018-06-01/runtime/invocation/err-1/error"),
                "unexpected request: {req}"
            );
            let ok = b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(ok).expect("write ack");
            let _ = stream.shutdown(Shutdown::Both);
        });

        let api = LambdaRuntimeApi::with_base(&base).expect("client");
        api.post_error("err-1", br#"{"error":"boom"}"#)
            .expect("error post");
        server.join().expect("server");
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // ss[verify graph.lambda.runtime-api]
    #[test]
    fn from_env_unset_is_err() {
        let _g = ENV_LOCK.lock().expect("env lock");
        let prev = std::env::var("AWS_LAMBDA_RUNTIME_API").ok();
        unsafe { std::env::remove_var("AWS_LAMBDA_RUNTIME_API") };
        let err = match LambdaRuntimeApi::from_env() {
            Err(e) => e,
            Ok(_) => panic!("from_env succeeded with AWS_LAMBDA_RUNTIME_API unset"),
        };
        assert!(err.contains("not set"), "{err}");
        if let Some(p) = prev {
            unsafe { std::env::set_var("AWS_LAMBDA_RUNTIME_API", p) };
        }
    }

    // ss[verify graph.lambda.runtime-api]
    #[test]
    fn from_env_host_port() {
        let _g = ENV_LOCK.lock().expect("env lock");
        let prev = std::env::var("AWS_LAMBDA_RUNTIME_API").ok();
        unsafe { std::env::set_var("AWS_LAMBDA_RUNTIME_API", "127.0.0.1:9001") };
        let api = LambdaRuntimeApi::from_env().expect("from_env");
        assert_eq!(api.url("/invocation/next"), "http://127.0.0.1:9001/2018-06-01/runtime/invocation/next");
        match prev {
            Some(p) => unsafe { std::env::set_var("AWS_LAMBDA_RUNTIME_API", p) },
            None => unsafe { std::env::remove_var("AWS_LAMBDA_RUNTIME_API") },
        }
    }

    // ss[verify graph.lambda.runtime-api]
    #[test]
    fn next_invocation_missing_request_id() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let base = format!("http://127.0.0.1:{port}");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let _ = read_http_request(&mut stream);
            let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
            stream.write_all(resp).expect("write");
            let _ = stream.shutdown(Shutdown::Both);
        });
        let api = LambdaRuntimeApi::with_base(&base).expect("client");
        let err = api.next_invocation().expect_err("missing header");
        assert!(err.contains("missing Lambda-Runtime-Aws-Request-Id"), "{err}");
        server.join().expect("server");
    }

    // ss[verify graph.lambda.runtime-api]
    #[test]
    fn get_and_post_connection_refused() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        let api = LambdaRuntimeApi::with_base(format!("http://127.0.0.1:{port}")).expect("client");
        let get_err = api.next_invocation().expect_err("GET refused");
        assert!(get_err.contains("GET next"), "{get_err}");
        let post_err = api.post_response("id", b"{}").expect_err("POST refused");
        assert!(post_err.contains("POST response"), "{post_err}");
        let err_post = api.post_error("id", b"{}").expect_err("POST error refused");
        assert!(err_post.contains("POST error"), "{err_post}");
    }

    use proptest::prelude::*;
    crate::ss_proptest! {
        /// Property: `with_base` strips trailing slashes from the origin.
        #[test]
        // ss[verify graph.lambda.runtime-api]
        // ss[verify verify.process.proptest]
        fn proptest_with_base_strips_trailing_slashes(slashes in 0usize..4) {
            let suffix = "/".repeat(slashes);
            let api = LambdaRuntimeApi::with_base(format!("http://127.0.0.1:9{}", suffix))
                .expect("client");
            let got = api.url("/invocation/next");
            prop_assert_eq!(
                got,
                "http://127.0.0.1:9/2018-06-01/runtime/invocation/next"
            );
        }
    }
}
