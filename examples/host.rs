use std::net::*;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() {
    env_logger::init();
    let server = Arc::new(usbip::UsbIpServer::new_from_host_with_filter(|d| {
        d.class() != 0x09 && d.vendor_id() == 0x0951
    }).await);
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 3240);
    tokio::spawn(usbip::server(addr, server));

    loop {
        // sleep 1s
        tokio::time::sleep(Duration::new(1, 0)).await;
    }
}
