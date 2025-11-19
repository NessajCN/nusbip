use env_logger;
use log::*;
use std::net::*;
use std::sync::Arc;
use tokio::signal;

#[tokio::main]
async fn main() {
    env_logger::init();
    let server = Arc::new(
        nusbip::UsbIpServer::new_from_host_with_filter(|d| {
            // Filter all mass storage devices (bInterfaceClass 08h for mass storage)
            // Caveat: Do NOT export all usb devices
            // unless you know exactly what you are doing.
            d.interfaces().any(|i| i.class() == 0x08)
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
            error!("Unable to listen for shutdown signal: {}", err);
            // we also shut down in case of error
            server.cleanup().await;
        }
    }
}
