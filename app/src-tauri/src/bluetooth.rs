//! Bluetooth: paired directly with Windows, the mouse offers only its battery level.
//! Buttons, DPI and the other settings stay with the receiver or the cable, so this is for display only.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Serialize;

const VENDOR_ID: u16 = 0x3554;
const CHECK: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BluetoothState {
    pub connected: bool,
    pub battery: Option<u8>,
}

#[derive(Clone, Default)]
pub struct BluetoothHandle(Arc<Mutex<BluetoothState>>);

impl BluetoothHandle {
    pub fn state(&self) -> BluetoothState {
        self.0.lock().unwrap().clone()
    }
}

/// Windows reports a change of the connection at once, the battery level is read every half minute.
/// The handle should already be managed, because `on_change` usually looks it up.
pub fn watch(handle: BluetoothHandle, on_change: impl Fn(BluetoothState) + Send + 'static) {
    thread::spawn(move || {
        let (wake, woken) = mpsc::channel();
        let mut watcher = le::Watcher::new(wake);
        loop {
            let state = watcher.read();
            let changed = {
                let mut shared = handle.0.lock().unwrap();
                let changed = *shared != state;
                *shared = state.clone();
                changed
            };
            if changed {
                on_change(state);
            }
            let _ = woken.recv_timeout(CHECK);
            while woken.try_recv().is_ok() {}
        }
    });
}

#[cfg(windows)]
mod le {
    use std::sync::mpsc::Sender;

    use windows::core::{Result, GUID};
    use windows::Devices::Bluetooth::GenericAttributeProfile::{GattCharacteristicUuids, GattCommunicationStatus, GattServiceUuids};
    use windows::Devices::Bluetooth::{BluetoothCacheMode, BluetoothConnectionStatus, BluetoothLEDevice};
    use windows::Devices::Enumeration::DeviceInformation;
    use windows::Foundation::TypedEventHandler;
    use windows::Storage::Streams::DataReader;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    use super::{BluetoothState, VENDOR_ID};

    pub struct Watcher {
        wake: Sender<()>,
        com_ready: bool,
        device: Option<BluetoothLEDevice>,
    }

    impl Watcher {
        pub fn new(wake: Sender<()>) -> Self {
            Watcher { wake, com_ready: false, device: None }
        }

        pub fn read(&mut self) -> BluetoothState {
            if !self.com_ready {
                // The result does not matter: an already initialised thread is fine too.
                let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                self.com_ready = true;
            }
            if self.device.is_none() {
                self.device = find_mouse().ok().flatten();
                if let Some(device) = &self.device {
                    let wake = self.wake.clone();
                    let _ = device.ConnectionStatusChanged(&TypedEventHandler::new(move |_, _| {
                        let _ = wake.send(());
                        Ok(())
                    }));
                }
            }
            let Some(device) = &self.device else { return BluetoothState::default() };
            match device.ConnectionStatus() {
                Ok(BluetoothConnectionStatus::Connected) => BluetoothState { connected: true, battery: battery_level(device).ok().flatten() },
                Ok(_) => BluetoothState::default(),
                Err(_) => {
                    // Searched again next time, for example after the mouse was removed in Windows.
                    self.device = None;
                    BluetoothState::default()
                }
            }
        }
    }

    /// The mouse calls itself only "Hypace@Lofree", so it is told apart by the vendor in its PnP ID.
    fn find_mouse() -> Result<Option<BluetoothLEDevice>> {
        let selector = BluetoothLEDevice::GetDeviceSelectorFromPairingState(true)?;
        for info in DeviceInformation::FindAllAsyncAqsFilter(&selector)?.join()? {
            let Ok(device) = BluetoothLEDevice::FromIdAsync(&info.Id()?)?.join() else { continue };
            let pnp = read_value(&device, GattServiceUuids::DeviceInformation()?, GattCharacteristicUuids::PnpId()?, BluetoothCacheMode::Cached);
            if let Ok(Some(pnp)) = pnp {
                if pnp.len() >= 3 && u16::from_le_bytes([pnp[1], pnp[2]]) == VENDOR_ID {
                    return Ok(Some(device));
                }
            }
        }
        Ok(None)
    }

    fn battery_level(device: &BluetoothLEDevice) -> Result<Option<u8>> {
        let value = read_value(device, GattServiceUuids::Battery()?, GattCharacteristicUuids::BatteryLevel()?, BluetoothCacheMode::Uncached)?;
        Ok(value.and_then(|bytes| bytes.first().copied()).filter(|&level| level <= 100))
    }

    /// The services come from the Windows cache, `mode` only decides about the value itself.
    fn read_value(device: &BluetoothLEDevice, service: GUID, characteristic: GUID, mode: BluetoothCacheMode) -> Result<Option<Vec<u8>>> {
        let services = device.GetGattServicesForUuidWithCacheModeAsync(service, BluetoothCacheMode::Cached)?.join()?;
        if services.Status()? != GattCommunicationStatus::Success {
            return Ok(None);
        }
        for service in services.Services()? {
            let value = read_from_service(&service, characteristic, mode);
            // An open service keeps a session to the mouse until it is closed.
            let _ = service.Close();
            if let Ok(Some(bytes)) = value {
                return Ok(Some(bytes));
            }
        }
        Ok(None)
    }

    fn read_from_service(
        service: &windows::Devices::Bluetooth::GenericAttributeProfile::GattDeviceService,
        characteristic: GUID,
        mode: BluetoothCacheMode,
    ) -> Result<Option<Vec<u8>>> {
        let found = service.GetCharacteristicsForUuidWithCacheModeAsync(characteristic, BluetoothCacheMode::Cached)?.join()?;
        if found.Status()? != GattCommunicationStatus::Success {
            return Ok(None);
        }
        for characteristic in found.Characteristics()? {
            let read = characteristic.ReadValueWithCacheModeAsync(mode)?.join()?;
            if read.Status()? != GattCommunicationStatus::Success {
                continue;
            }
            let buffer = read.Value()?;
            let mut bytes = vec![0u8; buffer.Length()? as usize];
            DataReader::FromBuffer(&buffer)?.ReadBytes(&mut bytes)?;
            return Ok(Some(bytes));
        }
        Ok(None)
    }
}

#[cfg(not(windows))]
mod le {
    use std::sync::mpsc::Sender;

    use super::BluetoothState;

    pub struct Watcher;

    impl Watcher {
        pub fn new(_wake: Sender<()>) -> Self {
            Watcher
        }

        pub fn read(&mut self) -> BluetoothState {
            BluetoothState::default()
        }
    }
}
