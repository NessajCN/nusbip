//! A library for running a USB/IP server

use log::*;
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use nusb::transfer::Direction;
use nusb::{DeviceInfo, Speed};
use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::io::{ErrorKind, Result};
use std::net::SocketAddr;
#[cfg(not(target_os = "macos"))]
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use usbip_protocol::UsbIpCommand;

pub mod cdc;
mod consts;
mod device;
mod endpoint;
pub mod hid;
mod host;
mod interface;
mod setup;
pub mod usbip_protocol;
mod util;
pub use consts::*;
pub use device::*;
pub use endpoint::*;
pub use host::*;
pub use interface::*;
pub use setup::*;
pub use util::*;

use crate::usbip_protocol::{USBIP_RET_SUBMIT, USBIP_RET_UNLINK, UsbIpHeaderBasic, UsbIpResponse};

/// Main struct of a USB/IP server
#[derive(Default, Debug)]
pub struct UsbIpServer {
    available_devices: RwLock<Vec<UsbDevice>>,
    used_devices: RwLock<Vec<UsbDevice>>,
}

impl UsbIpServer {
    /// Create a [UsbIpServer] with simulated devices
    pub fn new_simulated(devices: Vec<UsbDevice>) -> Self {
        Self {
            available_devices: RwLock::new(devices),
            used_devices: RwLock::new(Vec::new()),
        }
    }

    /// Create a [UsbIpServer] with Vec<[nusb::DeviceInfo]> for sharing host devices
    pub async fn with_nusb_devices(nusb_device_infos: Vec<nusb::DeviceInfo>) -> Vec<UsbDevice> {
        let mut devices = vec![];
        for device_info in nusb_device_infos {
            let dev = match device_info.open().await {
                Ok(dev) => dev,
                Err(err) => {
                    warn!("Impossible to open device {device_info:?}: {err}, ignoring device",);
                    continue;
                }
            };

            #[cfg(target_os = "linux")]
            let path = device_info.sysfs_path().to_path_buf();
            #[cfg(not(target_os = "linux"))]
            let path = device_info.bus_id().to_string();
            #[cfg(target_os = "linux")]
            let bus_id = match path.file_name() {
                Some(s) => s.to_os_string().into_string().unwrap_or(format!(
                    "{}-{}-{}",
                    device_info.busnum(),
                    device_info.device_address(),
                    0,
                )),
                None => format!(
                    "{}-{}-{}",
                    device_info.busnum(),
                    device_info.device_address(),
                    0,
                ),
            };
            #[cfg(not(target_os = "linux"))]
            let bus_id = device_info.bus_id().to_string();

            #[cfg(target_os = "linux")]
            let bus_num = device_info.busnum() as u32;
            #[cfg(not(target_os = "linux"))]
            let bus_num = 0u32;
            let cfg = match dev.active_configuration() {
                Ok(cfg) => cfg,
                Err(err) => {
                    warn!(
                        "Impossible to get active configuration {device_info:?}: {err}, ignoring device",
                    );
                    continue;
                }
            };
            let attributes = cfg.attributes();
            let max_power = cfg.max_power();
            let mut interfaces = vec![];
            for intf in cfg.interfaces() {
                // ignore alternate settings
                let intf_num = intf.interface_number();

                #[cfg(target_os = "linux")]
                let _ = dev.detach_kernel_driver(intf_num);

                let intf = dev.claim_interface(intf_num).await.unwrap();
                let intf_desc = intf.descriptor().unwrap();

                let mut endpoints = vec![];

                for ep_desc in intf_desc.endpoints() {
                    endpoints.push(UsbEndpoint {
                        address: ep_desc.address(),
                        attributes: ep_desc.transfer_type() as u8,
                        max_packet_size: ep_desc.max_packet_size() as u16,
                        interval: ep_desc.interval(),
                    });
                }

                let handler = intf.clone();

                interfaces.push(UsbInterface {
                    interface_class: intf_desc.class(),
                    interface_subclass: intf_desc.subclass(),
                    interface_protocol: intf_desc.protocol(),
                    endpoints,
                    string_interface: match intf_desc.string_index() {
                        Some(i) => i.into(),
                        None => 0,
                    },
                    class_specific_descriptor: Vec::new(),
                    handler,
                });
            }

            let speed = match device_info.speed() {
                Some(s) => match s {
                    Speed::Low => 1u32,
                    Speed::Full => 2,
                    Speed::High => 3,
                    Speed::Super => 5,
                    Speed::SuperPlus => 6,
                    _ => s as u32 + 1,
                },
                None => 0u32,
            };
            let mut device = UsbDevice {
                path,
                bus_id,
                bus_num,
                dev_num: device_info.device_address() as u32,
                speed,
                vendor_id: device_info.vendor_id(),
                product_id: device_info.product_id(),
                device_class: device_info.class(),
                device_subclass: device_info.subclass(),
                device_protocol: device_info.protocol(),
                device_bcd: device_info.device_version().into(),
                configuration_value: cfg.configuration_value(),
                num_configurations: dev.configurations().count() as u8,
                ep0_in: UsbEndpoint {
                    address: 0x80,
                    attributes: EndpointAttributes::Control as u8,
                    max_packet_size: EP0_MAX_PACKET_SIZE,
                    interval: 0,
                },
                ep0_out: UsbEndpoint {
                    address: 0x00,
                    attributes: EndpointAttributes::Control as u8,
                    max_packet_size: EP0_MAX_PACKET_SIZE,
                    interval: 0,
                },
                interfaces,
                device_handler: Some(dev),
                usb_version: device_info.usb_version().into(),
                attributes,
                max_power,
                ..UsbDevice::default()
            };

            // set strings
            if let Some(s) = device_info.manufacturer_string() {
                device.string_manufacturer = device.new_string(s)
            }
            if let Some(s) = device_info.product_string() {
                device.string_product = device.new_string(s)
            }
            if let Some(s) = device_info.serial_number() {
                device.string_serial = device.new_string(s)
            }
            devices.push(device);
        }
        devices
    }

    /// Create a [UsbIpServer] exposing devices in the host, and redirect all USB transfers to them using libusb
    pub async fn new_from_host() -> Self {
        Self::new_from_host_with_filter(|_| true).await
    }

    /// Create a [UsbIpServer] exposing filtered devices in the host, and redirect all USB transfers to them using libusb
    pub async fn new_from_host_with_filter<F>(filter: F) -> Self
    where
        F: FnMut(&DeviceInfo) -> bool,
    {
        match nusb::list_devices().await {
            Ok(list) => {
                let devs: Vec<DeviceInfo> = list.filter(filter).collect();
                // info!("devices: {devs:?}");
                Self {
                    available_devices: RwLock::new(Self::with_nusb_devices(devs).await),
                    ..Default::default()
                }
            }
            Err(_) => Default::default(),
        }
    }

    pub async fn add_device(&self, device: UsbDevice) {
        self.available_devices.write().await.push(device);
    }

    pub async fn remove_device(&self, bus_id: &str) -> Result<()> {
        let mut available_devices = self.available_devices.write().await;

        if let Some(i) = available_devices.iter().position(|d| d.bus_id == bus_id) {
            if let Some(dev) = available_devices[i].device_handler.clone() {
                release_claim(dev);
            }
            available_devices.remove(i);
            Ok(())
        } else if self
            .used_devices
            .read()
            .await
            .iter()
            .any(|d| d.bus_id == bus_id)
        {
            Err(std::io::Error::other(format!(
                "Device {} is in use",
                bus_id
            )))
        } else {
            Err(std::io::Error::new(
                ErrorKind::NotFound,
                format!("Device {bus_id} not found"),
            ))
        }
    }

    pub async fn occupy(&self, bus_id: &str) -> Result<UsbDevice> {
        let mut ad = self.available_devices.write().await;
        let device = match ad.iter().position(|d| d.bus_id == bus_id) {
            Some(i) => ad.remove(i),
            None => return Err(std::io::Error::other(format!("No available device"))),
        };
        let mut ud = self.used_devices.write().await;
        if !ud.iter().any(|d| d.bus_id == device.bus_id) {
            ud.push(device.clone());
        }
        Ok(device)
    }

    pub async fn release(&self, device: UsbDevice) {
        let mut ud = self.used_devices.write().await;
        let mut ad = self.available_devices.write().await;
        let new_vec = ud.clone();
        let new_ud: Vec<UsbDevice> = new_vec
            .into_iter()
            .filter(|d| d.bus_id != device.bus_id)
            .collect();
        if !ad.iter().any(|d| d.bus_id == device.bus_id) {
            ad.push(device);
        }
        *ud = new_ud;
    }

    /// Reclaim the detached os driver.
    pub async fn cleanup(&self) {
        let mut ud = self.used_devices.write().await;
        let mut ad = self.available_devices.write().await;
        for d in ud.clone() {
            if !ad.iter().any(|dev| d.bus_id == dev.bus_id) {
                ad.push(d);
            }
        }
        *ud = Vec::new();
        #[cfg(target_os = "linux")]
        {
            for d in ad.iter() {
                if let Some(dh) = d.device_handler.clone() {
                    release_claim(dh);
                }
            }
            *ad = Vec::new();
        }
    }

    pub async fn handle_op_req_devlist(&self) -> Result<UsbIpResponse> {
        trace!("Got OP_REQ_DEVLIST");
        let devices = self.available_devices.read().await;

        // OP_REP_DEVLIST
        let usbip_resp = UsbIpResponse::op_rep_devlist(&devices);
        trace!("Sent OP_REP_DEVLIST");
        Ok(usbip_resp)
    }

    pub async fn handle_op_req_import(
        &self,
        busid: [u8; 32],
        imported_device: &mut Option<UsbDevice>,
    ) -> Result<UsbIpResponse> {
        trace!("Got OP_REQ_IMPORT");

        let trimmed_busid = &busid[..busid.iter().position(|&x| x == 0).unwrap_or(busid.len())];
        let bus_id = match str::from_utf8(trimmed_busid) {
            Ok(s) => s,
            Err(_e) => return Err(std::io::Error::other(format!("Invalid bus id: {busid:?}"))),
        };

        match imported_device.take() {
            Some(dev) => self.release(dev).await,
            None => (),
        }

        let usbip_resp = match self.occupy(bus_id).await {
            Ok(dev) => {
                let res = UsbIpResponse::op_rep_import_success(&dev);
                *imported_device = Some(dev);
                res
            }
            Err(_) => UsbIpResponse::op_rep_import_fail(),
        };

        trace!("Sent OP_REP_IMPORT");
        Ok(usbip_resp)
    }

    pub fn handle_usbip_cmd_submit(
        &self,
        mut header: UsbIpHeaderBasic,
        transfer_buffer_length: u32,
        setup: [u8; 8],
        data: Vec<u8>,
        device: &UsbDevice,
    ) -> Result<UsbIpResponse> {
        let out = header.direction == 0;
        let real_ep = if out { header.ep } else { header.ep | 0x80 };

        header.command = USBIP_RET_SUBMIT.into();

        // Reply header from server should have devid/direction/ep all 0.
        header.devid = 0;
        header.direction = 0;
        header.ep = 0;

        let usbip_resp = match device.find_ep(real_ep as u8) {
            None => {
                warn!("Endpoint {real_ep:02x?} not found");
                UsbIpResponse::usbip_ret_submit_fail(&header, 0)
            }
            Some((ep, intf)) => {
                match device.handle_urb(
                    ep,
                    intf,
                    transfer_buffer_length,
                    SetupPacket::parse(&setup),
                    &data,
                ) {
                    Ok(resp) => {
                        if out {
                            trace!("<-Wrote {}", data.len());
                        } else {
                            trace!("<-Resp {resp:02x?}");
                        }
                        let actual_length = match ep.direction() {
                            Direction::In => resp.len() as u32,
                            Direction::Out => transfer_buffer_length,
                        };
                        UsbIpResponse::usbip_ret_submit_success(
                            &header,
                            0,
                            actual_length,
                            resp,
                            vec![],
                        )
                    }
                    Err(err) => {
                        warn!("Error handling URB: {err}");
                        let actual_length = match ep.direction() {
                            Direction::In => 0,
                            Direction::Out => transfer_buffer_length,
                        };
                        UsbIpResponse::usbip_ret_submit_fail(&header, actual_length)
                    }
                }
            }
        };
        trace!("Sent USBIP_RET_SUBMIT");
        Ok(usbip_resp)
    }

    pub fn handle_usbip_cmd_unlink(
        &self,
        mut header: UsbIpHeaderBasic,
        unlink_seqnum: u32,
    ) -> Result<UsbIpResponse> {
        trace!("Got USBIP_CMD_UNLINK for {unlink_seqnum:10x?}");

        header.command = USBIP_RET_UNLINK.into();
        // Reply header from server should have devid/direction/ep all 0.
        header.devid = 0;
        header.direction = 0;
        header.ep = 0;

        let res = UsbIpResponse::usbip_ret_unlink_success(&header);
        trace!("Sent USBIP_RET_UNLINK");
        Ok(res)
    }
}

pub async fn handler<T: AsyncReadExt + AsyncWriteExt + Unpin>(
    socket: &mut T,
    server: Arc<UsbIpServer>,
    imported_device: &mut Option<UsbDevice>,
) -> Result<()> {
    loop {
        let command = match UsbIpCommand::read_from_socket(socket).await {
            Ok(c) => c,
            Err(err) => {
                if let Some(dev) = imported_device.take() {
                    server.release(dev).await;
                }
                if err.kind() == ErrorKind::UnexpectedEof {
                    info!("Remote closed the connection");
                    return Ok(());
                } else {
                    return Err(err);
                }
            }
        };

        match command {
            UsbIpCommand::OpReqDevlist { .. } => match server.handle_op_req_devlist().await {
                Ok(r) => {
                    r.write_to_socket(socket).await?;
                }
                Err(e) => error!("UsbipCommand OpReqDevlist handling error: {e:?}"),
            },
            UsbIpCommand::OpReqImport { busid, .. } => {
                match server.handle_op_req_import(busid, imported_device).await {
                    Ok(r) => {
                        r.write_to_socket(socket).await?;
                    }
                    Err(e) => {
                        error!("UsbipCommand OpReqImport handling error: {e:?}");
                        if let Some(dev) = imported_device.take() {
                            server.release(dev).await;
                        }
                    }
                }
                info!("Imported device: {imported_device:?}");
            }
            UsbIpCommand::UsbIpCmdSubmit {
                header,
                transfer_buffer_length,
                setup,
                data,
                ..
            } => {
                let device = match imported_device.as_ref() {
                    Some(d) => d,
                    None => {
                        error!("No device currently imported");
                        continue;
                    }
                };
                match server.handle_usbip_cmd_submit(
                    header,
                    transfer_buffer_length,
                    setup,
                    data,
                    device,
                ) {
                    Ok(r) => {
                        r.write_to_socket(socket).await?;
                    }
                    Err(e) => error!("UsbipCmdSubmit handling error: {e:?}"),
                }
            }
            UsbIpCommand::UsbIpCmdUnlink {
                header,
                unlink_seqnum,
            } => match server.handle_usbip_cmd_unlink(header, unlink_seqnum) {
                Ok(r) => {
                    r.write_to_socket(socket).await?;
                }
                Err(e) => error!("UsbipCmdUnlink handling error: {e:?}"),
            },
        }
    }
}

/// Spawn a USB/IP server at `addr` using [TcpListener]
pub async fn server(addr: SocketAddr, server: Arc<UsbIpServer>) {
    let listener = TcpListener::bind(addr).await.expect("bind to addr");

    while let Ok((mut socket, _addr)) = listener.accept().await {
        info!("Got connection from {:?}", socket.peer_addr());
        let new_server = server.clone();
        tokio::spawn(async move {
            let mut imported_device: Box<Option<UsbDevice>> = Box::new(None);
            let res = handler(&mut socket, new_server.clone(), &mut imported_device).await;
            info!("Handler ended with {res:?}");
            if let Some(dev) = imported_device.take() {
                new_server.release(dev).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use tokio::{net::TcpStream, task::JoinSet};

    use super::*;
    use crate::{
        usbip_protocol::{USBIP_CMD_SUBMIT, UsbIpHeaderBasic},
        util::tests::*,
    };

    const SINGLE_DEVICE_BUSID: &str = "0-0-0";

    fn op_req_import(busid: &str) -> Vec<u8> {
        let mut busid = busid.to_string().as_bytes().to_vec();
        busid.resize(32, 0);
        UsbIpCommand::OpReqImport {
            status: 0,
            busid: busid.try_into().unwrap(),
        }
        .to_bytes()
    }

    async fn attach_device(connection: &mut TcpStream, busid: &str) -> u32 {
        let req = op_req_import(busid);
        connection.write_all(req.as_slice()).await.unwrap();
        connection.read_u32().await.unwrap();
        let result = connection.read_u32().await.unwrap();
        if result == 0 {
            connection.read_exact(&mut vec![0; 0x138]).await.unwrap();
        }
        result
    }

    #[tokio::test]
    async fn req_empty_devlist() {
        setup_test_logger();
        let server = UsbIpServer::new_simulated(vec![]);
        let req = UsbIpCommand::OpReqDevlist { status: 0 };
        let mut imported_device: Box<Option<UsbDevice>> = Box::new(None);
        let mut mock_socket = MockSocket::new(req.to_bytes());
        handler(&mut mock_socket, Arc::new(server), &mut imported_device)
            .await
            .ok();

        assert_eq!(
            mock_socket.output,
            UsbIpResponse::op_rep_devlist(&[]).to_bytes(),
        );
    }

    #[tokio::test]
    async fn add_and_remove_10_devices() {
        setup_test_logger();
        let server_ = Arc::new(UsbIpServer::new_simulated(vec![]));
        let addr = get_free_address().await;
        tokio::spawn(server(addr, server_.clone()));

        let mut join_set = JoinSet::new();
        let devices = (0..10).map(UsbDevice::new).collect::<Vec<_>>();

        for device in devices.iter() {
            let new_server = server_.clone();
            let new_device = device.clone();
            join_set.spawn(async move {
                new_server.add_device(new_device).await;
            });
        }

        for device in devices.iter() {
            let new_server = server_.clone();
            let new_device = device.clone();
            join_set.spawn(async move {
                new_server.remove_device(&new_device.bus_id).await.unwrap();
            });
        }

        while join_set.join_next().await.is_some() {}

        let device_len = server_.clone().available_devices.read().await.len();

        assert_eq!(device_len, 0);
    }
}
