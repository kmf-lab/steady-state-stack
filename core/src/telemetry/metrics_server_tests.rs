//! Tests extracted from `metrics_server.rs` so the production file stays under the 1,200-line budget.
//! Nested test mods keep their own imports.

#[cfg(test)]
// ss[impl telemetry.builtin-server]
mod meteric_server_tests {



    #[test]
    #[cfg(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))]
    // ss[verify telemetry.builtin-server]
    fn test_simple() -> Result<(), Box<dyn std::error::Error>> {
        // ss[related philosophy.structural-hierarchy]
        use crate::{ActorIdentity, GraphBuilder, SoloAct};
        // ss[related philosophy.structural-hierarchy]
        use std::sync::Arc;
        // ss[impl telemetry.builtin-server]
        use std::thread::{sleep};
        // ss[related philosophy.structural-hierarchy]
        use std::time::Duration;
        // ss[related philosophy.structural-hierarchy]
        use crate::monitor::ActorMetaData;
        // ss[impl telemetry.builtin-server]
        use crate::telemetry::metrics_collector::DiagramData;
        // ss[related philosophy.structural-hierarchy]
        use crate::telemetry::metrics_server::internal_behavior;

        let mut graph = GraphBuilder::for_testing().build(());

        let (tx_in, rx_in) = graph.channel_builder()
             .with_capacity(10).build_channel();

        let rate_ms = graph.telemetry_production_rate_ms;

        graph.actor_builder()
            .with_name("UnitTest")
            .build(move |context| internal_behavior(context, rate_ms, rx_in.clone(), None, None, 4)
                   , SoloAct);
 
        let test_data:Vec<DiagramData> = (0..3).map(|i| DiagramData::NodeDef( i
                 , Box::new((
                    Arc::new(ActorMetaData{
                        ident: ActorIdentity::new(i as usize, "test_actor", None ),
                        ..Default::default() }), Box::new([]),Box::new([])
                ) ) )).collect();

        graph.start();
        tx_in.testing_send_all(test_data, true);

        sleep(Duration::from_millis(60));
        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(15))
    
     }

}

#[cfg(not(windows))]
#[cfg(test)]
#[cfg(all(feature = "prometheus_metrics", feature = "telemetry_server_builtin"))]
// ss[impl telemetry.builtin-server]
mod http_telemetry_tests {
    // ss[related philosophy.structural-hierarchy]
    use std::io::Read;
    // ss[related philosophy.structural-hierarchy]
    use std::thread::sleep;
    // ss[impl telemetry.builtin-server]
    use super::super::*;
    // ss[related philosophy.structural-hierarchy]
    use crate::GraphBuilder;
    // ss[related philosophy.structural-hierarchy]
    use std::time::Duration;
    
    // ss[impl telemetry.builtin-server]
    use crate::monitor::ActorStatus;

    #[test]
    #[cfg(all(feature="prometheus_metrics",feature="telemetry_server_builtin"))]
    // ss[verify telemetry.builtin-server]
    fn test_metrics_server() -> Result<(), Box<dyn std::error::Error>> {
        if cfg!(not(windows)) && std::env::var("GITHUB_ACTIONS").is_err() {
            let (mut graph, server_ip, tx_in) = stand_up_test_server("127.0.0.1:0");

            // Capture and validate the metrics server content
            // Fetch the metrics from the server
            // trace!("Server location {:?}",server_ip);
            if let Some(ref addr) = server_ip {
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, Some("digraph"), "graph.dot");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, Some("Helvetica, Arial, Liberation Sans, sans-serif"), "dot-viewer.css");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, Some("'1 sec': 1000,"), "dot-viewer.js");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                // ss[verify telemetry.live-title]
                validate_path(&addr, Some("setTelemetryTitle"), "dot-viewer.js");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                // ss[verify telemetry.live-title]
                validate_path(&addr, Some("Live Telemetry"), "dot-viewer.js");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                // ss[verify telemetry.live-title]
                validate_path(&addr, Some("Snapshot"), "dot-viewer.js");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                // ss[verify telemetry.live-title]
                validate_path(&addr, Some("ok: true"), "webworker.js");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                // ss[verify telemetry.live-title]
                validate_path(&addr, Some("ok: false"), "webworker.js");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, Some("refresh_rate_ms"), "config");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, Some("pendingUrl"), "webworker.js");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, Some("this.importScripts('viz-lite.js');"), "webworker.js");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, Some("<title>Telemetry</title>"), "index.html");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, Some("id=\"telemetryTitle\""), "index.html");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, Some("Loading…"), "index.html");
                print!(".");
                // Regression: HTTP 200 with an empty or corrupt embedded gzip once broke browser
                // `importScripts` for viz-lite — `validate_path(..., None)` did not catch it.
                #[cfg(feature = "telemetry_server_builtin")]
                validate_viz_lite_js_gzip_payload(addr);
                print!(".");
                #[cfg(feature = "prometheus_metrics")]
                validate_path(&addr, Some("="), "metric");
                print!(".");

                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "images/preview-icon.svg");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "images/refresh-time-icon.svg");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "images/user-icon.svg");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "images/zoom-in-icon.svg");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "images/zoom-in-icon-disabled.svg");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "images/zoom-out-icon.svg");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "images/zoom-out-icon-disabled.svg");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "images/download.png");
                print!(".");

            } else {
                panic!("Telemetry address not available");
            }
            // Step 6: Stop the graph
            tx_in.testing_close();
            graph.request_shutdown();
            graph.block_until_stopped(Duration::from_secs(5))
        } else {
            Ok(())
        }
    }

    /// Checks if an address can be bound to and returns the local address if successful.
    // ss[impl telemetry.builtin-server]
    #[cfg(all(feature = "prometheus_metrics", feature = "telemetry_server_builtin"))]
    pub(crate) fn check_addr(addr: &str) -> Option<String> {
        if let Ok(h) = TcpListener::bind(addr) {
            let local_addr = h.local_addr().expect("Unable to get local address");
            Some(format!("{}", local_addr))
        } else {
            None
        }
    }

    // ss[impl telemetry.builtin-server]
    #[cfg(all(feature = "prometheus_metrics", feature = "telemetry_server_builtin"))]
    fn stand_up_test_server(addr: &str) -> (Graph, Option<String>, LazySteadyTx<DiagramData>) {
        // Step 1: Set up a minimal graph
        let mut graph = GraphBuilder::for_testing()
            .with_telemtry_production_rate_ms(500)
            .build(());

        // Step 2: Start the metrics_server actor        
        let (tx_in, rx_in) = graph.channel_builder().build_channel();
        if let Some(ref addr) = Some(addr.to_string()) {
            if let Some(addr) = check_addr(addr) {
                println!("{}",&addr);                
                launch_server(graph, Some(addr), tx_in, rx_in)
            } else {
                panic!("Unable to Bind to http://{}", addr);
            }
        } else {
            panic!("Unable to Bind to http://{:?}", &addr);
        }        
    }

    // ss[impl telemetry.builtin-server]
    #[cfg(all(feature = "prometheus_metrics", feature = "telemetry_server_builtin"))]
    fn launch_server(mut graph: Graph, server_ip: Option<String>, tx_in: LazySteadyTx<DiagramData>
                           , rx_in: LazySteadyRx<DiagramData>) -> (Graph, Option<String>, LazySteadyTx<DiagramData>) {

        let server_ip_out = server_ip.clone();
        graph.actor_builder()
            .with_name("metrics_server")
            .build(move |context| {
                let frame_rate_ms = context.frame_rate_ms;
                internal_behavior(context, frame_rate_ms, rx_in.clone(), server_ip.clone(), None, 4)
            },SoloAct);

        // Step 3: Start the graph
        graph.start();

        // Allow the server to start
        sleep(Duration::from_millis(500));

        // Step 4: Send test data to the metrics_server
        // Simulate DiagramData messages
        let sequence = 0;
        let mut data: Vec<DiagramData> = (0..4).map(|i| DiagramData::NodeDef(
            sequence,
            Box::new((
                Arc::new(ActorMetaData {
                    ident: ActorIdentity::new(i, "test_actor", None),
                    ..Default::default()
                }),
                Box::new([]),
                Box::new([]),
            )),
        )).collect();

        let node_status: Vec<ActorStatus> = (0..4).map(|_i|
            ActorStatus {
                await_total_ns: 100,
                unit_total_ns: 200,
                total_count_restarts: 1,
                iteration_start: 0,
                iteration_sum: 0,
                bool_stop: false,
                is_quiet: false,
                calls: [0; 6],
                thread_info: None,
                bool_blocking: false,
                ident: ActorIdentity::new(0, "test_actor", None),
            }
        ).collect();

        data.push(DiagramData::NodeProcessData(0, node_status.clone().into()));
        data.push(DiagramData::ChannelVolumeData(0, vec![(0, 5, 10), (1, 15, 20)].into()));
        data.push(DiagramData::NodeProcessData(1, node_status.into()));
        data.push(DiagramData::ChannelVolumeData(1, vec![(0, 15, 20), (1, 30, 30)].into()));

        tx_in.testing_send_all(data, false);
        (graph, server_ip_out, tx_in)
    }

    // ss[impl telemetry.builtin-server]
    #[cfg(all(feature = "prometheus_metrics", feature = "telemetry_server_builtin"))]
    fn validate_path(addr: &&String, expected_text: Option<&str>, path: &str) {
        match isahc::get(format!("http://{}/{}", &addr, &path)) {
            Ok(response) => {
                assert_eq!(response.status(), 200);

                let mut body = response.into_body();

                // Read and validate the response body
                let mut text = String::new();
                let _ = body.read_to_string(&mut text);

                if let Some(expected_text) = expected_text {
                    if !text.trim().is_empty() {
                        assert!(
                            text.contains(expected_text),
                            "not found {} in {}", expected_text,
                            text
                        );
                    }
                }
            },
            Err(_) => {
                info!("unable to test port: {}",&addr);
            }
        };
    }

    /// Decodes `/viz-lite.js` (gzip on the wire) and asserts the script is non-trivial.
    ///
    /// A prior bug embedded an empty or wrong gzip while still returning HTTP 200; browsers then
    /// failed `importScripts` for viz-lite. Plain `validate_path(..., None)` did not catch that.
    #[cfg(feature = "telemetry_server_builtin")]
    // ss[impl telemetry.builtin-server]
    fn validate_viz_lite_js_gzip_payload(addr: &str) {
        // ss[related philosophy.structural-hierarchy]
        use flate2::read::GzDecoder;
        // ss[related philosophy.structural-hierarchy]
        use std::io::Read;

        // ss[impl telemetry.builtin-server]
        const MIN_DECODED_SCRIPT_BYTES: usize = 5000;

        let url = format!("http://{}/viz-lite.js", addr);
        let response = isahc::get(url).expect("GET viz-lite.js");
        assert_eq!(response.status(), 200);
        let mut raw = Vec::new();
        response
            .into_body()
            .read_to_end(&mut raw)
            .expect("read viz-lite.js body");
        assert!(
            !raw.is_empty(),
            "viz-lite.js body empty"
        );
        // `isahc` may transparently gunzip even when the server sets `Content-Encoding: gzip`; handle
        // both raw gzip on the wire and already-decoded bodies.
        let decoded = if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
            let mut decoder = GzDecoder::new(&raw[..]);
            let mut s = String::new();
            decoder
                .read_to_string(&mut s)
                .expect("gunzip viz-lite.js");
            s
        } else {
            String::from_utf8_lossy(&raw).into_owned()
        };
        assert!(
            decoded.len() >= MIN_DECODED_SCRIPT_BYTES,
            "viz-lite.js decoded script unexpectedly small: {} bytes",
            decoded.len()
        );
        assert!(
            decoded.contains("Viz") && decoded.contains("function"),
            "decoded viz-lite.js missing expected markers (len {})",
            decoded.len()
        );
    }

}

#[cfg(test)]
// ss[impl telemetry.builtin-server]
mod handle_request_logic_tests {
    // ss[related philosophy.structural-hierarchy]
    use super::super::*;
    // ss[related philosophy.structural-hierarchy]
    use futures::io::{AsyncRead, AsyncWrite};
    // ss[impl telemetry.builtin-server]
    use std::pin::Pin;
    // ss[related philosophy.structural-hierarchy]
    use std::sync::Mutex;
    // ss[related philosophy.structural-hierarchy]
    use std::task::{Context, Poll};
    // ss[related philosophy.structural-hierarchy]
    use std::io;

    /// Serializes `TELEMETRY_SERVER_PORT` mutation across parallel unit tests.
    static BIND_TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

    // ss[impl telemetry.builtin-server]
    struct MockStream {
        read_data: Vec<u8>,
        read_pos: usize,
        write_data: Vec<u8>,
    }

    // ss[impl telemetry.builtin-server]
    impl MockStream {
        // ss[related philosophy.structural-hierarchy]
        fn new(data: &str) -> Self {
            MockStream {
                read_data: data.as_bytes().to_vec(),
                read_pos: 0,
                write_data: Vec::new(),
            }
        }
    }

    // ss[impl telemetry.builtin-server]
    impl AsyncRead for MockStream {
        // ss[related philosophy.structural-hierarchy]
        fn poll_read(mut self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
            let remaining = self.read_data.len() - self.read_pos;
            let n = remaining.min(buf.len());
            buf[..n].copy_from_slice(&self.read_data[self.read_pos..self.read_pos + n]);
            self.read_pos += n;
            Poll::Ready(Ok(n))
        }
    }

    // ss[impl telemetry.builtin-server]
    impl AsyncWrite for MockStream {
        // ss[related philosophy.structural-hierarchy]
        fn poll_write(mut self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
            self.write_data.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }
        // ss[impl telemetry.builtin-server]
        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> { Poll::Ready(Ok(())) }
        // ss[related philosophy.structural-hierarchy]
        fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> { Poll::Ready(Ok(())) }
    }

    #[test]
    // ss[verify telemetry.builtin-server]
    fn test_handle_request_index() {
        let state = Arc::new(RwLock::new(MetricState { doc: Bytes::new(), metric: Bytes::new(), config: Bytes::new(), last_disk_write: Arc::new(AtomicU64::new(0)), start_time: Instant::now() }));
        let stream = MockStream::new("GET / HTTP/1.1\r\n\r\n");
        futures::executor::block_on(handle_request(stream, state)).unwrap();
    }

    #[test]
    // ss[verify telemetry.builtin-server]
    fn test_handle_request_options() {
        let state = Arc::new(RwLock::new(MetricState { doc: Bytes::new(), metric: Bytes::new(), config: Bytes::new(), last_disk_write: Arc::new(AtomicU64::new(0)), start_time: Instant::now() }));
        let stream = MockStream::new("OPTIONS / HTTP/1.1\r\n\r\n");
        futures::executor::block_on(handle_request(stream, state)).unwrap();
    }

    #[test]
    // ss[verify telemetry.builtin-server]
    fn test_handle_request_404() {
        let state = Arc::new(RwLock::new(MetricState { doc: Bytes::new(), metric: Bytes::new(), config: Bytes::new(), last_disk_write: Arc::new(AtomicU64::new(0)), start_time: Instant::now() }));
        let stream = MockStream::new("GET /unknown HTTP/1.1\r\n\r\n");
        futures::executor::block_on(handle_request(stream, state)).unwrap();
    }

    #[test]
    // ss[verify telemetry.builtin-server]
    fn test_handle_request_assets() {
        let state = Arc::new(RwLock::new(MetricState { doc: Bytes::new(), metric: Bytes::new(), config: Bytes::new(), last_disk_write: Arc::new(AtomicU64::new(0)), start_time: Instant::now() }));
        let paths = [
            "/",
            "/index.html",
            "/graph.dot",
            "/metrics",
            "/images/zoom-in-icon.svg",
            "/images/zoom-in-icon-disabled.svg",
            "/images/zoom-out-icon.svg",
            "/images/zoom-out-icon-disabled.svg",
            "/images/refresh-time-icon.svg",
            "/images/user-icon.svg",
            "/images/preview-icon.svg",
            "/images/spinner.gif",
            "/images/download.png",
            "/webworker.js",
            "/dot-viewer.css",
            "/dot-viewer.js",
            "/viz-lite.js",
            "/unknown",
        ];
        for path in paths {
            let request = format!("GET {} HTTP/1.1\r\n\r\n", path);
            let stream = MockStream::new(&request);
            let _ = futures::executor::block_on(handle_request(stream, state.clone()));
        }
    }

    #[test]
    // ss[verify telemetry.builtin-server]
    fn test_handle_request_graph_dot_with_payload() {
        let state = Arc::new(RwLock::new(MetricState {
            doc: Bytes::from_static(b"digraph G { a -> b }"),
            metric: Bytes::new(),
            config: Bytes::new(),
            last_disk_write: Arc::new(AtomicU64::new(0)),
            start_time: Instant::now(),
        }));
        let stream = MockStream::new("GET /graph.dot HTTP/1.1\r\n\r\n");
        futures::executor::block_on(handle_request(stream, state)).expect("graph.dot response");
    }

    #[test]
    // ss[verify telemetry.builtin-server]
    fn test_bind_to_port_invalid_address_returns_none() {
        let listener = bind_to_port("not-a-valid-host:0");
        assert!(listener.listener.as_ref().is_none());
    }

    #[test]
    // ss[verify telemetry.builtin-server]
    fn bind_to_port_finds_next_port_when_default_in_use() {
        let _guard = BIND_TEST_ENV_LOCK
            .lock()
            .expect("bind test env lock");
        unsafe {
            std::env::remove_var("TELEMETRY_SERVER_PORT");
        }
        // Hold 9900 when free; an external telemetry instance may already occupy it.
        let hold = TcpListener::bind("127.0.0.1:9900");
        const DEFAULT_PORT: u16 = 9900;

        let result = bind_to_port("127.0.0.1:9900");
        assert!(
            result.listener.as_ref().is_some(),
            "expected fallback bind when default port scan enabled"
        );
        let bound = result
            .listener
            .as_ref()
            .as_ref()
            .expect("listener")
            .local_addr()
            .expect("bound addr");
        if hold.is_ok() {
            assert_eq!(bound.port(), DEFAULT_PORT + 1);
        } else {
            assert!(
                bound.port() > DEFAULT_PORT,
                "when default port is externally occupied, scan must bind above {DEFAULT_PORT}, got {}",
                bound.port()
            );
        }
        assert_eq!(result.requested_port, DEFAULT_PORT);
    }

    #[test]
    // ss[verify telemetry.builtin-server]
    fn bind_to_port_fail_fast_when_env_port_in_use() {
        let _guard = BIND_TEST_ENV_LOCK
            .lock()
            .expect("bind test env lock");
        let hold = TcpListener::bind("127.0.0.1:0").expect("hold listener");
        let blocked_port = hold.local_addr().expect("local addr").port();
        unsafe {
            std::env::set_var("TELEMETRY_SERVER_PORT", blocked_port.to_string());
        }

        let result = bind_to_port(&format!("127.0.0.1:{blocked_port}"));
        assert!(
            result.listener.as_ref().is_none(),
            "explicit env port should fail fast without scanning"
        );

        unsafe {
            std::env::remove_var("TELEMETRY_SERVER_PORT");
        }
    }

    #[test]
    // ss[verify telemetry.builtin-server]
    fn bind_to_port_port_zero_single_attempt() {
        let _guard = BIND_TEST_ENV_LOCK
            .lock()
            .expect("bind test env lock");
        unsafe {
            std::env::set_var("TELEMETRY_SERVER_PORT", "0");
        }

        let result = bind_to_port("127.0.0.1:0");
        assert!(
            result.listener.as_ref().is_some(),
            "ephemeral port 0 should bind without scanning"
        );

        unsafe {
            std::env::remove_var("TELEMETRY_SERVER_PORT");
        }
    }

    #[test]
    // ss[verify telemetry.builtin-server]
    #[cfg(feature = "prometheus_metrics")]
    // ss[related philosophy.structural-hierarchy]
    fn test_handle_request_metrics_with_payload() {
        let state = Arc::new(RwLock::new(MetricState {
            doc: Bytes::from_static(b"digraph {}"),
            metric: Bytes::from_static(b"steady_up 1\n"),
            config: Bytes::from_static(b"{\"refresh_rate_ms\":40}"),
            last_disk_write: Arc::new(AtomicU64::new(0)),
            start_time: Instant::now(),
        }));
        let stream = MockStream::new("GET /metrics HTTP/1.1\r\n\r\n");
        futures::executor::block_on(handle_request(stream, state)).expect("metrics response");
    }

    #[test]
    // ss[verify telemetry.builtin-server]
    fn test_async_write_all_writes_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("graph.dot");
        let file = std::fs::File::create(&path).expect("create file");
        futures::executor::block_on(async_write_all(
            BytesMut::from(&b"graph contents"[..]),
            true,
            file,
        ))
        .expect("write");
        let written = std::fs::read_to_string(path).expect("read back");
        assert_eq!(written, "graph contents");
    }
}
