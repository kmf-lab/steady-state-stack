
use tide::prelude::*; // Pulls in the json! macro.
use tide_websockets::{Message, WebSocket};
use crate::steady::{SteadyMonitor, SteadyRx};
use crate::steady::telemetry::metrics_collector::DiagramData;
use async_std::stream::StreamExt;
use tide::{Endpoint, Request};
use crate::steady::*;

/*
// Define an async function to handle WebSocket connections.
async fn handle_ws(request: Request<()>) -> tide::Result<()> {
    WebSocket::new(|_req, mut stream| async move {
        while let Some(Ok(Message::Text(input))) = stream.next().await {
            // Echo the message back
            stream.send_string(input).await?;
        }
        Ok(())
    })
        .call(request)
        .await
}

async fn example() -> tide::Result<()> {
    let mut app = tide::new();

    // Add a route that listens for GET requests and upgrades to WebSocket.
    app.at("/ws").get(handle_ws);

    // Start the server
    app.listen("127.0.0.1:8080").await?;
    Ok(())
}
*/

pub(crate) async fn telemetry(p0: SteadyMonitor, p1: SteadyRx<DiagramData>) -> Result<(),()> {
    Ok(())
}