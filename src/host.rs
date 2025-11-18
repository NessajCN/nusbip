//! Host USB
use log::*;
use nusb::{
    Device, Interface, MaybeFuture,
    transfer::{Buffer, Bulk, Direction, In, Interrupt, Out},
};
use rusb::{DeviceHandle, GlobalContext};
use std::io::Result;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::{any::Any, time::Duration};

use crate::{
    EndpointAttributes, SetupPacket, UsbDeviceHandler, UsbEndpoint, UsbInterface,
    UsbInterfaceHandler,
};

/// A handler to pass requests to interface of a rusb USB device of the host
#[derive(Clone, Debug)]
pub struct RusbUsbHostInterfaceHandler {
    handle: Arc<Mutex<DeviceHandle<GlobalContext>>>,
}

impl RusbUsbHostInterfaceHandler {
    pub fn new(handle: Arc<Mutex<DeviceHandle<GlobalContext>>>) -> Self {
        Self { handle }
    }
}

impl UsbInterfaceHandler for RusbUsbHostInterfaceHandler {
    fn handle_urb(
        &mut self,
        _interface: &UsbInterface,
        ep: UsbEndpoint,
        transfer_buffer_length: u32,
        setup: SetupPacket,
        req: &[u8],
    ) -> Result<Vec<u8>> {
        debug!("To host device: ep={ep:?} setup={setup:?} req={req:?}",);
        let mut buffer = vec![0u8; transfer_buffer_length as usize];
        let timeout = std::time::Duration::new(1, 0);
        let handle = self.handle.lock().unwrap();
        if ep.attributes == EndpointAttributes::Control as u8 {
            // control
            if let Direction::In = ep.direction() {
                // control in
                if let Ok(len) = handle.read_control(
                    setup.request_type,
                    setup.request,
                    setup.value,
                    setup.index,
                    &mut buffer,
                    timeout,
                ) {
                    return Ok(Vec::from(&buffer[..len]));
                }
            } else {
                // control out
                handle
                    .write_control(
                        setup.request_type,
                        setup.request,
                        setup.value,
                        setup.index,
                        req,
                        timeout,
                    )
                    .ok();
            }
        } else if ep.attributes == EndpointAttributes::Interrupt as u8 {
            // interrupt
            if let Direction::In = ep.direction() {
                // interrupt in
                if let Ok(len) = handle.read_interrupt(ep.address, &mut buffer, timeout) {
                    info!("intr in {:?}", &buffer[..len]);
                    return Ok(Vec::from(&buffer[..len]));
                }
            } else {
                // interrupt out
                handle.write_interrupt(ep.address, req, timeout).ok();
            }
        } else if ep.attributes == EndpointAttributes::Bulk as u8 {
            // bulk
            if let Direction::In = ep.direction() {
                // bulk in
                if let Ok(len) = handle.read_bulk(ep.address, &mut buffer, timeout) {
                    return Ok(Vec::from(&buffer[..len]));
                }
            } else {
                // bulk out
                handle.write_bulk(ep.address, req, timeout).ok();
            }
        }
        Ok(vec![])
    }

    fn get_class_specific_descriptor(&self) -> Vec<u8> {
        vec![]
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

/// A handler to pass requests to device of a rusb USB device of the host
#[derive(Clone, Debug)]
pub struct RusbUsbHostDeviceHandler {
    handle: Arc<Mutex<DeviceHandle<GlobalContext>>>,
}

impl RusbUsbHostDeviceHandler {
    pub fn new(handle: Arc<Mutex<DeviceHandle<GlobalContext>>>) -> Self {
        Self { handle }
    }
}

impl UsbDeviceHandler for RusbUsbHostDeviceHandler {
    fn handle_urb(
        &mut self,
        transfer_buffer_length: u32,
        setup: SetupPacket,
        req: &[u8],
    ) -> Result<Vec<u8>> {
        debug!("To host device: setup={setup:?} req={req:?}");
        let mut buffer = vec![0u8; transfer_buffer_length as usize];
        let timeout = std::time::Duration::new(1, 0);
        let handle = self.handle.lock().unwrap();
        // control
        if setup.request_type & 0x80 == 0 {
            // control out
            handle
                .write_control(
                    setup.request_type,
                    setup.request,
                    setup.value,
                    setup.index,
                    req,
                    timeout,
                )
                .ok();
        } else {
            // control in
            if let Ok(len) = handle.read_control(
                setup.request_type,
                setup.request,
                setup.value,
                setup.index,
                &mut buffer,
                timeout,
            ) {
                return Ok(Vec::from(&buffer[..len]));
            }
        }
        Ok(vec![])
    }

    #[cfg(target_os = "linux")]
    fn release_claim(&mut self) {}

    #[cfg(not(target_os = "windows"))]
    fn reset(&mut self) -> Result<()> {
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    fn set_configuration(&self, setup: &[u8; 8]) -> Result<()> {
        Ok(())
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

/// A handler to pass requests to interface of a nusb USB device of the host
#[derive(Clone)]
pub struct NusbUsbHostInterfaceHandler {
    handle: nusb::Interface,
}

impl std::fmt::Debug for NusbUsbHostInterfaceHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NusbUsbHostInterfaceHandler")
            .field("handle", &"Opaque")
            .finish()
    }
}

impl NusbUsbHostInterfaceHandler {
    pub fn new(handle: nusb::Interface) -> Self {
        Self { handle }
    }
}

impl UsbInterfaceHandler for NusbUsbHostInterfaceHandler {
    fn handle_urb(
        &mut self,
        _interface: &UsbInterface,
        ep: UsbEndpoint,
        transfer_buffer_length: u32,
        setup: SetupPacket,
        req: &[u8],
    ) -> Result<Vec<u8>> {
        let mut buffer = vec![0u8; transfer_buffer_length as usize];
        let timeout = std::time::Duration::new(1, 0);
        let handle = self.handle.clone();
        // let control = nusb::transfer::ControlIn {
        //     control_type: match (setup.request_type >> 5) & 0b11 {
        //         0 => nusb::transfer::ControlType::Standard,
        //         1 => nusb::transfer::ControlType::Class,
        //         2 => nusb::transfer::ControlType::Vendor,
        //         _ => unimplemented!(),
        //     },
        //     recipient: match setup.request_type & 0b11111 {
        //         0 => nusb::transfer::Recipient::Device,
        //         1 => nusb::transfer::Recipient::Interface,
        //         2 => nusb::transfer::Recipient::Endpoint,
        //         3 => nusb::transfer::Recipient::Other,
        //         _ => unimplemented!(),
        //     },
        //     request: setup.request,
        //     value: setup.value,
        //     index: setup.index,
        // };
        if ep.attributes == EndpointAttributes::Control as u8 {
            // control
            if let Direction::In = ep.direction() {
                // control in
                let control = nusb::transfer::ControlIn {
                    control_type: match (setup.request_type >> 5) & 0b11 {
                        0 => nusb::transfer::ControlType::Standard,
                        1 => nusb::transfer::ControlType::Class,
                        2 => nusb::transfer::ControlType::Vendor,
                        _ => unimplemented!(),
                    },
                    recipient: match setup.request_type & 0b11111 {
                        0 => nusb::transfer::Recipient::Device,
                        1 => nusb::transfer::Recipient::Interface,
                        2 => nusb::transfer::Recipient::Endpoint,
                        3 => nusb::transfer::Recipient::Other,
                        _ => unimplemented!(),
                    },
                    request: setup.request,
                    value: setup.value,
                    index: setup.index,
                    length: setup.length,
                };
                if let Ok(buf) = handle.control_in(control, timeout).wait() {
                    return Ok(buf);
                }
            } else {
                // control out
                let control = nusb::transfer::ControlOut {
                    control_type: match (setup.request_type >> 5) & 0b11 {
                        0 => nusb::transfer::ControlType::Standard,
                        1 => nusb::transfer::ControlType::Class,
                        2 => nusb::transfer::ControlType::Vendor,
                        _ => unimplemented!(),
                    },
                    recipient: match setup.request_type & 0b11111 {
                        0 => nusb::transfer::Recipient::Device,
                        1 => nusb::transfer::Recipient::Interface,
                        2 => nusb::transfer::Recipient::Endpoint,
                        3 => nusb::transfer::Recipient::Other,
                        _ => unimplemented!(),
                    },
                    request: setup.request,
                    value: setup.value,
                    index: setup.index,
                    data: req,
                };
                handle.control_out(control, timeout).wait()?;
            }
        } else if ep.attributes == EndpointAttributes::Interrupt as u8 {
            // interrupt
            // todo!("Missing blocking api for interrupt transfer in nusb")
            if let Direction::In = ep.direction() {
                // interrupt in
                let mut reader = handle
                    .endpoint::<Interrupt, In>(ep.address)?
                    .reader(4096)
                    .with_read_timeout(timeout);

                if let Ok(()) = reader.read_exact(&mut buffer) {
                    info!("interrupt in {:?}", &buffer);
                    return Ok(buffer);
                }
            } else {
                // interrupt out
                let mut writer = handle
                    .endpoint::<Interrupt, Out>(ep.address)?
                    .writer(4096)
                    .with_write_timeout(timeout);
                writer.write_all(&req)?;
                writer.flush()?;
            }
        } else if ep.attributes == EndpointAttributes::Bulk as u8 {
            // bulk
            // todo!("Missing blocking api for bulk transfer in nusb")
            if let Direction::In = ep.direction() {
                // bulk in
                let mut reader = handle
                    .endpoint::<Bulk, In>(ep.address)?
                    .reader(4096)
                    .with_read_timeout(timeout);

                match reader.read_exact(&mut buffer) {
                    Ok(()) => {
                        info!("intr in {:02x?}", &buffer);
                        return Ok(buffer);
                    }
                    Err(e) => {
                        error!("Error when read buffer: {e:?}");
                        return Err(e);
                    }
                }
                // if let Ok(len) = handle.read_bulk(ep.address, &mut buffer, timeout) {
                //     return Ok(Vec::from(&buffer[..len]));
                // }
            } else {
                // bulk out
                let mut writer = handle
                    .endpoint::<Bulk, Out>(ep.address)?
                    .writer(4096)
                    .with_write_timeout(timeout);
                writer.write_all(&req)?;
                writer.flush()?;
                // handle.write_bulk(ep.address, req, timeout).ok();
            }
        }
        Ok(vec![])
    }

    fn get_class_specific_descriptor(&self) -> Vec<u8> {
        vec![]
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

pub fn handle_urb_for_interface(
    interface: Interface,
    // device: Device,
    ep: UsbEndpoint,
    transfer_buffer_length: u32,
    setup: SetupPacket,
    req: &[u8],
) -> Result<Vec<u8>> {
    let timeout = Duration::new(1, 0);
    // info!(
    //     "Handling interface with endpoint: {ep:?}, interface: {}, transfer length: {transfer_buffer_length}",
    //     interface.interface_number()
    // );
    if ep.attributes == EndpointAttributes::Control as u8 {
        // control
        let control_type = match (setup.request_type >> 5) & 0b11 {
            0 => nusb::transfer::ControlType::Standard,
            1 => nusb::transfer::ControlType::Class,
            2 => nusb::transfer::ControlType::Vendor,
            _ => unimplemented!(),
        };
        let recipient = match setup.request_type & 0b11111 {
            0 => nusb::transfer::Recipient::Device,
            1 => nusb::transfer::Recipient::Interface,
            2 => nusb::transfer::Recipient::Endpoint,
            3 => nusb::transfer::Recipient::Other,
            _ => unimplemented!(),
        };
        if let Direction::In = ep.direction() {
            // control in
            let control = nusb::transfer::ControlIn {
                control_type,
                recipient,
                request: setup.request,
                value: setup.value,
                // For Recipient::Interface this is the interface number. For Recipient::Endpoint this is the endpoint number.
                index: setup.index,
                length: setup.length,
            };
            // info!(
            //     "Control in command received, setup: {setup:?}, \nreq: {req:02x?},\ncontrol: {control:02x?}"
            // );

            if let Ok(buf) = interface.control_in(control, timeout).wait() {
                return Ok(buf);
            }
        } else {
            // control out
            let control = nusb::transfer::ControlOut {
                control_type,
                recipient,
                request: setup.request,
                value: setup.value,
                // For Recipient::Interface this is the interface number. For Recipient::Endpoint this is the endpoint number.
                index: setup.index,
                data: req,
            };
            // info!(
            //     "Control out command received, setup: {setup:?}, \nreq: {req:02x?},\ncontrol: {control:02x?}"
            // );
            interface.control_out(control, timeout).wait()?;
        }
    // } else if setup.is_setup() {
    //     if setup.request_type >> 7 == 1 {
    //         // control in
    //         let control = nusb::transfer::ControlIn {
    //             control_type: match (setup.request_type >> 5) & 0b11 {
    //                 0 => nusb::transfer::ControlType::Standard,
    //                 1 => nusb::transfer::ControlType::Class,
    //                 2 => nusb::transfer::ControlType::Vendor,
    //                 _ => unimplemented!(),
    //             },
    //             recipient: match setup.request_type & 0b11111 {
    //                 0 => nusb::transfer::Recipient::Device,
    //                 1 => nusb::transfer::Recipient::Interface,
    //                 2 => nusb::transfer::Recipient::Endpoint,
    //                 3 => nusb::transfer::Recipient::Other,
    //                 _ => unimplemented!(),
    //             },
    //             request: setup.request,
    //             value: setup.value,
    //             index: setup.index,
    //             length: setup.length,
    //         };
    //         if let Ok(buf) = interface.control_in(control, timeout).await {
    //             return Ok(buf);
    //         }
    //     } else {
    //         // control out
    //         let control = nusb::transfer::ControlOut {
    //             control_type: match (setup.request_type >> 5) & 0b11 {
    //                 0 => nusb::transfer::ControlType::Standard,
    //                 1 => nusb::transfer::ControlType::Class,
    //                 2 => nusb::transfer::ControlType::Vendor,
    //                 _ => unimplemented!(),
    //             },
    //             recipient: match setup.request_type & 0b11111 {
    //                 0 => nusb::transfer::Recipient::Device,
    //                 1 => nusb::transfer::Recipient::Interface,
    //                 2 => nusb::transfer::Recipient::Endpoint,
    //                 3 => nusb::transfer::Recipient::Other,
    //                 _ => unimplemented!(),
    //             },
    //             request: setup.request,
    //             value: setup.value,
    //             index: setup.index,
    //             data: req,
    //         };
    //         interface.control_out(control, timeout).await?;
    //     }
    } else if ep.attributes == EndpointAttributes::Interrupt as u8 {
        // interrupt
        // todo!("Missing blocking api for interrupt transfer in nusb")
        if let Direction::In = ep.direction() {
            // interrupt in
            let mut reader = interface
                .endpoint::<Interrupt, In>(ep.address)?
                .reader(4096)
                .with_num_transfers(1)
                .with_read_timeout(timeout);
            let mut buffer = vec![0u8; transfer_buffer_length as usize];

            if let Ok(()) = reader.read_exact(&mut buffer) {
                // info!("interrupt in {:?}", &buffer[..len]);
                return Ok(buffer);
            }
        } else {
            // interrupt out
            let mut writer = interface
                .endpoint::<Interrupt, Out>(ep.address)?
                .writer(4096)
                .with_num_transfers(1)
                .with_write_timeout(timeout);
            writer.write_all(&req)?;
            writer.flush()?;
        }
    } else if ep.attributes == EndpointAttributes::Bulk as u8 {
        // bulk
        // todo!("Missing blocking api for bulk transfer in nusb")
        // if setup.is_setup() {
        //     if setup.request_type >> 7 == 1 {
        //         // control in
        //         let control = nusb::transfer::ControlIn {
        //             control_type: match (setup.request_type >> 5) & 0b11 {
        //                 0 => nusb::transfer::ControlType::Standard,
        //                 1 => nusb::transfer::ControlType::Class,
        //                 2 => nusb::transfer::ControlType::Vendor,
        //                 _ => unimplemented!(),
        //             },
        //             recipient: match setup.request_type & 0b11111 {
        //                 0 => nusb::transfer::Recipient::Device,
        //                 1 => nusb::transfer::Recipient::Interface,
        //                 2 => nusb::transfer::Recipient::Endpoint,
        //                 3 => nusb::transfer::Recipient::Other,
        //                 _ => unimplemented!(),
        //             },
        //             request: setup.request,
        //             value: setup.value,
        //             index: setup.index,
        //             length: setup.length,
        //         };
        //         if let Err(e) = interface.control_in(control, timeout).await {
        //             warn!("Error on control in : {e:?}");
        //         }
        //     } else {
        //         // control out
        //         let control = nusb::transfer::ControlOut {
        //             control_type: match (setup.request_type >> 5) & 0b11 {
        //                 0 => nusb::transfer::ControlType::Standard,
        //                 1 => nusb::transfer::ControlType::Class,
        //                 2 => nusb::transfer::ControlType::Vendor,
        //                 _ => unimplemented!(),
        //             },
        //             recipient: match setup.request_type & 0b11111 {
        //                 0 => nusb::transfer::Recipient::Device,
        //                 1 => nusb::transfer::Recipient::Interface,
        //                 2 => nusb::transfer::Recipient::Endpoint,
        //                 3 => nusb::transfer::Recipient::Other,
        //                 _ => unimplemented!(),
        //             },
        //             request: setup.request,
        //             value: setup.value,
        //             index: setup.index,
        //             data: req,
        //         };
        //         if let Err(e) = interface.control_out(control, timeout).await {
        //             warn!("Error on control out transfer: {e:?}");
        //         }
        //     }
        // }
        if let Direction::In = ep.direction() {
            // bulk in
            // #[cfg(target_os = "linux")]
            // match device.detach_kernel_driver(interface.interface_number()) {
            //     Ok(()) => info!("Kernal driver detached at {}", interface.interface_number()),
            //     Err(e) => error!(
            //         "Failed to detach kernel driver: {e:?}, interface num : {}",
            //         interface.interface_number()
            //     ),
            // }
            // let interface = device
            //     .claim_interface(interface.interface_number())
            //     .await
            //     .unwrap();
            let mut ep_in = interface.endpoint::<Bulk, In>(ep.address)?;
            let max_packet_size = ep_in.max_packet_size();

            let requested_len =
                ((transfer_buffer_length - 1) as usize / max_packet_size + 1) * max_packet_size;
            let buffer = Buffer::new(requested_len);
            let c = ep_in.transfer_blocking(buffer, timeout);
            let buf = c.into_result()?;
            return Ok(buf.into_vec());
            // let mut reader = ep_in
            //     .reader(4096)
            //     .with_num_transfers(1)
            //     .with_read_timeout(timeout);
            // // let mut buffer = vec![0u8; max_packet_size];

            // let mut buffer = vec![0u8; transfer_buffer_length as usize];
            // match reader.read_exact(&mut buffer) {
            //     Ok(_) => {
            //         // info!("Reading bulk in {:02x?},  ep: {ep:02x?}", &buffer);
            //         return Ok(buffer);
            //     }
            //     Err(e) => {
            //         error!("Error when read buffer: {e:?}, buffer: {buffer:02x?}.");
            //         return Err(e);
            //     }
            // }
            // if let Ok(len) = handle.read_bulk(ep.address, &mut buffer, timeout) {
            //     return Ok(Vec::from(&buffer[..len]));
            // }
        } else {
            // bulk out
            let mut writer = interface
                .endpoint::<Bulk, Out>(ep.address)?
                .writer(4096)
                .with_num_transfers(1)
                .with_write_timeout(timeout);
            // info!("Writing bulk out buffer {req:02x?}, ep: {ep:02x?}");
            writer.write_all(&req)?;
            writer.flush()?;
            // handle.write_bulk(ep.address, req, timeout).ok();
        }
    } else {
        warn!("Other command received, setup: {setup:?}, \nreq: {req:02x?},\ncontrol: {ep:02x?}");
    }
    Ok(vec![])
}

/// A handler to pass requests to device of a nusb USB device of the host
#[derive(Clone)]
pub struct NusbUsbHostDeviceHandler {
    handle: Arc<Mutex<nusb::Device>>,
}

impl std::fmt::Debug for NusbUsbHostDeviceHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NusbUsbHostDeviceHandler")
            .field("handle", &"Opaque")
            .finish()
    }
}

impl NusbUsbHostDeviceHandler {
    pub fn new(handle: Arc<Mutex<nusb::Device>>) -> Self {
        Self { handle }
    }
}

impl UsbDeviceHandler for NusbUsbHostDeviceHandler {
    fn handle_urb(
        &mut self,
        _transfer_buffer_length: u32,
        setup: SetupPacket,
        req: &[u8],
    ) -> Result<Vec<u8>> {
        // info!("To host device: setup={setup:?} req={req:?}");
        // let mut buffer = vec![0u8; transfer_buffer_length as usize];
        let timeout = std::time::Duration::new(1, 0);
        let handle = self.handle.lock().unwrap();
        // control
        if cfg!(not(target_os = "windows")) {
            if setup.request_type & 0x80 == 0 {
                // control out
                #[cfg(not(target_os = "windows"))]
                let control = nusb::transfer::ControlOut {
                    control_type: match (setup.request_type >> 5) & 0b11 {
                        0 => nusb::transfer::ControlType::Standard,
                        1 => nusb::transfer::ControlType::Class,
                        2 => nusb::transfer::ControlType::Vendor,
                        _ => unimplemented!(),
                    },
                    recipient: match setup.request_type & 0b11111 {
                        0 => nusb::transfer::Recipient::Device,
                        1 => nusb::transfer::Recipient::Interface,
                        2 => nusb::transfer::Recipient::Endpoint,
                        3 => nusb::transfer::Recipient::Other,
                        _ => unimplemented!(),
                    },
                    request: setup.request,
                    value: setup.value,
                    index: setup.index,
                    data: req,
                };
                handle.control_out(control, timeout).wait()?;
            } else {
                // control in
                #[cfg(not(target_os = "windows"))]
                let control = nusb::transfer::ControlIn {
                    control_type: match (setup.request_type >> 5) & 0b11 {
                        0 => nusb::transfer::ControlType::Standard,
                        1 => nusb::transfer::ControlType::Class,
                        2 => nusb::transfer::ControlType::Vendor,
                        _ => unimplemented!(),
                    },
                    recipient: match setup.request_type & 0b11111 {
                        0 => nusb::transfer::Recipient::Device,
                        1 => nusb::transfer::Recipient::Interface,
                        2 => nusb::transfer::Recipient::Endpoint,
                        3 => nusb::transfer::Recipient::Other,
                        _ => unimplemented!(),
                    },
                    request: setup.request,
                    value: setup.value,
                    index: setup.index,
                    length: setup.length,
                };

                if let Ok(buf) = handle.control_in(control, timeout).wait() {
                    return Ok(buf);
                }
            }
        } else {
            warn!("Not supported in windows")
        }
        Ok(vec![])
    }

    #[cfg(target_os = "linux")]
    fn release_claim(&mut self) {
        let dev = self.handle.lock().unwrap();
        let cfg = match dev.active_configuration() {
            Ok(cfg) => cfg,
            Err(err) => {
                warn!("Impossible to get active configuration: {err}, ignoring device",);
                return;
            }
        };
        for intf in cfg.interfaces() {
            // ignore alternate settings
            let intf_num = intf.interface_number();
            let _ = dev.attach_kernel_driver(intf_num);
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn reset(&mut self) -> Result<()> {
        let mut dev = self.handle.lock().unwrap();
        let vid = dev.device_descriptor().vendor_id();
        dev.reset().wait()?;
        let devices = nusb::list_devices().wait()?;
        match devices.into_iter().find(|d| d.vendor_id() == vid) {
            Some(device) => match device.open().wait() {
                Ok(d) => {
                    *dev = d;
                }
                Err(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "Cannot open device",
                    ));
                }
            },
            None => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Device not found",
                ));
            }
        }
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    fn set_configuration(&self, setup: &[u8; 8]) -> Result<()> {
        let dev = self.handle.lock().unwrap();
        let sp = SetupPacket::parse(setup);

        let cfg = dev.active_configuration()?;
        // info!("Interface cfg: {cfg:?}");

        // for intf in cfg.interfaces() {
        //     // ignore alternate settings
        //     let intf_num = intf.interface_number();
        //     #[cfg(target_os = "linux")]
        //     let _intf = match dev.detach_and_claim_interface(intf_num).wait() {
        //         Ok(i) => i,
        //         Err(e) => {
        //             error!("Interface claimed: {e:?}");
        //             return Err(e.into());
        //         }
        //     };
        //     #[cfg(not(target_os = "linux"))]
        //     let _intf = dev.claim_interface(intf_num).wait()?;
        // }

        dev.set_configuration(sp.value as u8).wait()?;
        Ok(())
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

pub fn handle_urb_for_device(
    device: Device,
    _transfer_buffer_length: u32,
    setup: SetupPacket,
    req: &[u8],
) -> Result<Vec<u8>> {
    // info!("To host device: setup={setup:?} req={req:?}");
    // let mut buffer = vec![0u8; transfer_buffer_length as usize];
    let timeout = std::time::Duration::new(1, 0);
    // control
    if cfg!(not(target_os = "windows")) {
        let control_type = match (setup.request_type >> 5) & 0b11 {
            0 => nusb::transfer::ControlType::Standard,
            1 => nusb::transfer::ControlType::Class,
            2 => nusb::transfer::ControlType::Vendor,
            _ => unimplemented!(),
        };
        let recipient = match setup.request_type & 0b11111 {
            0 => nusb::transfer::Recipient::Device,
            1 => nusb::transfer::Recipient::Interface,
            2 => nusb::transfer::Recipient::Endpoint,
            3 => nusb::transfer::Recipient::Other,
            _ => unimplemented!(),
        };
        if setup.request_type & 0x80 == 0 {
            // control out
            #[cfg(not(target_os = "windows"))]
            let control = nusb::transfer::ControlOut {
                control_type,
                recipient,
                request: setup.request,
                value: setup.value,
                index: setup.index,
                data: req,
            };
            device.control_out(control, timeout).wait()?;
        } else {
            // control in
            #[cfg(not(target_os = "windows"))]
            let control = nusb::transfer::ControlIn {
                control_type,
                recipient,
                request: setup.request,
                value: setup.value,
                index: setup.index,
                length: setup.length,
            };

            if let Ok(buf) = device.control_in(control, timeout).wait() {
                return Ok(buf);
            }
        }
    } else {
        warn!("Not supported in windows")
    }
    Ok(vec![])
}
