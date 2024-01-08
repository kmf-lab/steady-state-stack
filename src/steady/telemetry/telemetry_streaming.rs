
use crate::steady::{SteadyMonitor, SteadyRx};
use crate::steady::telemetry::metrics_collector::DiagramData;
use async_std::stream::StreamExt;


use tide::{Endpoint};
use tide_websockets::{Message, WebSocket};

// Define an async function to handle WebSocket connections.
async fn handle_ws(request: tide::Request<()>) -> tide::Result {
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



pub(crate) async fn run(mut monitor: SteadyMonitor, rx: SteadyRx<DiagramData>) -> std::result::Result<(),()> {

    //     monitor.init_stats(&[&rx], &[]); //TODO: this is not needed for this actor


    //TODO: stream dot file data to be constructed on the client end.
    //      this code will be shared with the history replay.
    //      we may want to also stream the in memory history data to the client


    let mut app = tide::new();

    // Add a route that listens for GET requests and upgrades to WebSocket.
    app.at("/ws").get(handle_ws);

    let server_future = app.listen("127.0.0.1:8080");

    // Start the server
    match server_future.await {
        Ok(_) => {},
        Err(_) => {},
    }

     // websocat is a command line tool for connecting to WebSockets servers
     // cargo install websocat
     // websocat ws://127.0.0.1:8080/ws
/*
    select! {
        _ = server_future.fuse() => {
            log::info!("Websocket server exited.");
        },
        _ = async_std::task::sleep(Duration::from_secs(10)) => {
            log::info!("Websocket server timed out.");
        },
        //we can read from the rx channel here.
        //TODO: we want the handle_ws outsourced to another
        //     actor child group so we can scale that up as needed
        //   this actor will be singular due to its need to hold the port.
     }
    */



    while rx.has_message() {
        match monitor.rx(&rx).await {
         //   Ok(DiagramData::Structure()) => {
       //         log::info!("Got DiagramData::Structure()");
       //     },
      //      Ok(DiagramData::Content()) => {
//info!("Got DiagramData::Content()");
       //     },
            _ => {}
        }
    }



    Ok(())
}




