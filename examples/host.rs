use env_logger;
use std::net::*;
use std::sync::Arc;
use tokio::signal;

#[tokio::main]
async fn main() {
    env_logger::init();
    let server = Arc::new(
        nusbip::UsbIpServer::new_from_host_with_filter(|d| {
            d.class() != 0x09 && d.vendor_id() == 0x0951
        })
        .await,
    );
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 3240);
    tokio::spawn(nusbip::server(addr, server.clone()));

    match signal::ctrl_c().await {
        Ok(()) => {
            server.cleanup().await;
        }
        Err(err) => {
            eprintln!("Unable to listen for shutdown signal: {}", err);
            // we also shut down in case of error
        }
    }
}
