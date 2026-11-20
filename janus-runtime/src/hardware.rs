//! Hardware abstraction layer for Titan device integration.

use anyhow::Result;
use serde::Serialize;
use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
    time::Duration,
    path::{Path, PathBuf},
    fs,
};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Honest adapter status returned to authenticated clients.
#[derive(Debug, Clone, Serialize)]
pub struct HardwareAdapterStatus {
    pub adapter: String,
    pub state: String,
    pub detail: String,
}

/// Hardware interface trait
pub trait HardwareInterface: Send + Sync {
    /// Get interface name
    fn name(&self) -> &str;

    /// Check if interface is available
    fn available(&self) -> bool;
}

/// Read-only USB inventory entry. Device serial numbers are deliberately excluded.
#[derive(Debug, Clone, Serialize)]
pub struct UsbDeviceDescriptor {
    pub sysfs_name: String,
    pub bus_number: Option<u16>,
    pub device_number: Option<u16>,
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
}

fn read_usb_attribute(directory: &Path, name: &str) -> Option<String> {
    fs::read_to_string(directory.join(name))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn parse_usb_hex(directory: &Path, name: &str) -> Option<u16> {
    u16::from_str_radix(read_usb_attribute(directory, name)?.trim_start_matches("0x"), 16).ok()
}

fn discover_usb_devices_at(root: &Path) -> Result<Vec<UsbDeviceDescriptor>> {
    let mut devices = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let directory = entry.path();
        if !directory.is_dir() {
            continue;
        }
        let (Some(vendor_id), Some(product_id)) = (
            parse_usb_hex(&directory, "idVendor"),
            parse_usb_hex(&directory, "idProduct"),
        ) else {
            continue;
        };
        devices.push(UsbDeviceDescriptor {
            sysfs_name: entry.file_name().to_string_lossy().into_owned(),
            bus_number: read_usb_attribute(&directory, "busnum")
                .and_then(|value| value.parse().ok()),
            device_number: read_usb_attribute(&directory, "devnum")
                .and_then(|value| value.parse().ok()),
            vendor_id,
            product_id,
            manufacturer: read_usb_attribute(&directory, "manufacturer"),
            product: read_usb_attribute(&directory, "product"),
        });
    }
    devices.sort_by(|left, right| {
        left.bus_number
            .cmp(&right.bus_number)
            .then_with(|| left.device_number.cmp(&right.device_number))
            .then_with(|| left.sysfs_name.cmp(&right.sysfs_name))
    });
    Ok(devices)
}

/// Read-only inventory entry for a serial device; serial numbers are intentionally omitted.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredSerialPort {
    pub port_name: String,
    pub transport: String,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
}

fn describe_serial_port(port: serialport::SerialPortInfo) -> DiscoveredSerialPort {
    match port.port_type {
        serialport::SerialPortType::UsbPort(usb) => DiscoveredSerialPort {
            port_name: port.port_name,
            transport: "usb".into(),
            vendor_id: Some(usb.vid),
            product_id: Some(usb.pid),
            manufacturer: usb.manufacturer,
            product: usb.product,
        },
        serialport::SerialPortType::PciPort => DiscoveredSerialPort {
            port_name: port.port_name,
            transport: "pci".into(),
            vendor_id: None,
            product_id: None,
            manufacturer: None,
            product: None,
        },
        serialport::SerialPortType::BluetoothPort => DiscoveredSerialPort {
            port_name: port.port_name,
            transport: "bluetooth".into(),
            vendor_id: None,
            product_id: None,
            manufacturer: None,
            product: None,
        },
        serialport::SerialPortType::Unknown => DiscoveredSerialPort {
            port_name: port.port_name,
            transport: "unknown".into(),
            vendor_id: None,
            product_id: None,
            manufacturer: None,
            product: None,
        },
    }
}

/// Serial port interface
pub struct SerialInterface {
    port: String,
    connection: Option<Arc<Mutex<Box<dyn serialport::SerialPort>>>>,
    last_error: Option<String>,
}

impl SerialInterface {
    pub fn new(port: impl Into<String>) -> Self {
        Self {
            port: port.into(),
            connection: None,
            last_error: None,
        }
    }

    /// Open a configured serial device at 115200 baud with bounded IO timeouts.
    pub async fn open(&mut self) -> Result<()> {
        let port_name = self.port.clone();
        let result = tokio::task::spawn_blocking(move || {
            serialport::new(port_name.clone(), 115_200)
                .timeout(Duration::from_millis(500))
                .open()
                .map(|port| Arc::new(Mutex::new(port)))
                .map_err(|error| anyhow::anyhow!("failed to open serial port {port_name}: {error}"))
        })
        .await?;

        match result {
            Ok(connection) => {
                self.connection = Some(connection);
                self.last_error = None;
                Ok(())
            }
            Err(error) => {
                self.connection = None;
                self.last_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    /// Write bytes to the connected serial device. No connection means an error.
    pub async fn write(&self, data: &[u8]) -> Result<usize> {
        let connection = self
            .connection
            .clone()
            .ok_or_else(|| anyhow::anyhow!("serial port {} is not open", self.port))?;
        let data = data.to_vec();
        tokio::task::spawn_blocking(move || {
            let mut port = connection
                .lock()
                .map_err(|_| anyhow::anyhow!("serial port lock is poisoned"))?;
            port.write_all(&data)?;
            Ok(data.len())
        })
        .await?
    }

    /// Read up to the supplied buffer length; the serial driver's timeout bounds waiting.
    pub async fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let connection = self
            .connection
            .clone()
            .ok_or_else(|| anyhow::anyhow!("serial port {} is not open", self.port))?;
        let capacity = buf.len();
        let (count, data) = tokio::task::spawn_blocking(move || {
            let mut data = vec![0; capacity];
            let mut port = connection
                .lock()
                .map_err(|_| anyhow::anyhow!("serial port lock is poisoned"))?;
            let count = port.read(&mut data)?;
            Ok::<_, anyhow::Error>((count, data))
        })
        .await??;
        buf[..count].copy_from_slice(&data[..count]);
        Ok(count)
    }
}

impl HardwareInterface for SerialInterface {
    fn name(&self) -> &str {
        "serial"
    }

    fn available(&self) -> bool {
        self.connection.is_some()
    }
}

/// GPIO interface
pub struct GpioInterface {
    pins: Vec<u32>,
}

impl GpioInterface {
    pub fn new() -> Self {
        Self { pins: vec![] }
    }

    pub async fn set_pin(&mut self, pin: u32, level: bool) -> Result<()> {
        let _ = (pin, level);
        anyhow::bail!("GPIO adapter is not implemented; no pin state was changed")
    }

    pub async fn read_pin(&self, pin: u32) -> Result<bool> {
        let _ = pin;
        anyhow::bail!("GPIO adapter is not implemented; no pin state was read")
    }
}

impl HardwareInterface for GpioInterface {
    fn name(&self) -> &str {
        "gpio"
    }

    fn available(&self) -> bool {
        false
    }
}

/// Cellular interface
pub struct CellularInterface;

impl CellularInterface {
    pub fn new() -> Self {
        Self
    }

    pub async fn scan(&self) -> Result<Vec<CellularNetwork>> {
        anyhow::bail!("cellular adapter is not implemented; no network scan was performed")
    }
}

impl HardwareInterface for CellularInterface {
    fn name(&self) -> &str {
        "cellular"
    }

    fn available(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct CellularNetwork {
    pub name: String,
    pub signal_strength: i32,
}

/// RF (Radio Frequency) interface
pub struct RfInterface;

impl RfInterface {
    pub fn new() -> Self {
        Self
    }

    pub async fn scan_frequencies(&self, start: u32, end: u32) -> Result<Vec<RfSignal>> {
        let _ = (start, end);
        anyhow::bail!("RF adapter is not implemented; no frequency scan was performed")
    }
}

impl HardwareInterface for RfInterface {
    fn name(&self) -> &str {
        "rf"
    }

    fn available(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct RfSignal {
    pub frequency: u32,
    pub strength: i32,
    pub modulation: String,
}

/// Sensor interface (accelerometer, GPS, etc.)
pub struct SensorInterface;

impl SensorInterface {
    pub fn new() -> Self {
        Self
    }

    pub async fn read_accelerometer(&self) -> Result<(f64, f64, f64)> {
        anyhow::bail!("accelerometer adapter is not implemented; no sensor reading is available")
    }

    pub async fn read_gps(&self) -> Result<(f64, f64)> {
        anyhow::bail!("GPS adapter is not implemented; no location reading is available")
    }
}

impl HardwareInterface for SensorInterface {
    fn name(&self) -> &str {
        "sensors"
    }

    fn available(&self) -> bool {
        false
    }
}

/// Hardware manager coordinating all interfaces
pub struct HardwareManager {
    enabled: bool,
    serial_configured: bool,
    serial: Arc<RwLock<SerialInterface>>,
    gpio: Arc<RwLock<GpioInterface>>,
    cellular: Arc<RwLock<CellularInterface>>,
    rf: Arc<RwLock<RfInterface>>,
    sensors: Arc<RwLock<SensorInterface>>,
}

impl HardwareManager {
    pub fn new(serial_port: Option<String>, enabled: bool) -> Self {
        let serial_configured = serial_port.is_some();
        let port = serial_port.unwrap_or_else(|| "/dev/ttyUSB0".to_string());
        Self {
            enabled,
            serial_configured,
            serial: Arc::new(RwLock::new(SerialInterface::new(port))),
            gpio: Arc::new(RwLock::new(GpioInterface::new())),
            cellular: Arc::new(RwLock::new(CellularInterface::new())),
            rf: Arc::new(RwLock::new(RfInterface::new())),
            sensors: Arc::new(RwLock::new(SensorInterface::new())),
        }
    }

    /// Enumerate USB devices from Linux sysfs without opening or probing them.
    pub async fn discover_usb_devices(&self) -> Result<Vec<UsbDeviceDescriptor>> {
        if !self.enabled {
            anyhow::bail!("hardware integration is disabled by configuration");
        }
        tokio::task::spawn_blocking(|| discover_usb_devices_at(Path::new("/sys/bus/usb/devices")))
            .await?
    }

    /// Enumerate serial endpoints without opening, probing, or transmitting to devices.
    pub async fn discover_serial_ports(&self) -> Result<Vec<DiscoveredSerialPort>> {
        if !self.enabled {
            anyhow::bail!("hardware integration is disabled by configuration");
        }
        tokio::task::spawn_blocking(|| {
            serialport::available_ports()
                .map(|ports| ports.into_iter().map(describe_serial_port).collect())
                .map_err(anyhow::Error::from)
        })
        .await?
    }

    pub async fn initialize(&self) -> Result<()> {
        if !self.enabled {
            info!("Hardware integration is disabled by configuration");
            return Ok(());
        }
        if self.serial_configured {
            let mut serial = self.serial.write().await;
            if let Err(error) = serial.open().await {
                warn!(%error, "Configured serial adapter could not be opened; runtime will continue without it");
            }
        } else {
            info!("No serial port configured; serial adapter will remain inactive");
        }
        info!("Hardware manager initialized; unimplemented adapters remain unsupported");
        Ok(())
    }

    /// Report only verified adapter availability; unsupported adapters never claim readiness.
    pub async fn status(&self) -> Vec<HardwareAdapterStatus> {
        let serial = self.serial.read().await;
        let other_state = if self.enabled { "unsupported" } else { "disabled" };
        let disabled_detail = "Hardware integration is disabled by system configuration.";
        let serial_state = if !self.enabled {
            "disabled"
        } else if !self.serial_configured {
            "not_configured"
        } else if serial.available() {
            "available"
        } else {
            "unavailable"
        };
        let serial_detail = if !self.enabled {
            disabled_detail.to_string()
        } else if !self.serial_configured {
            "No serial port path was configured.".to_string()
        } else if serial.available() {
            format!("Connected to configured serial device: {}", serial.port)
        } else {
            serial.last_error.clone().unwrap_or_else(|| {
                format!("Configured serial device {} has not been opened.", serial.port)
            })
        };
        vec![
            HardwareAdapterStatus {
                adapter: "serial".into(),
                state: serial_state.into(),
                detail: serial_detail,
            },
            HardwareAdapterStatus {
                adapter: "gpio".into(),
                state: other_state.into(),
                detail: if self.enabled { "No production GPIO driver is installed.".into() } else { disabled_detail.into() },
            },
            HardwareAdapterStatus {
                adapter: "cellular".into(),
                state: other_state.into(),
                detail: if self.enabled { "No production cellular adapter is installed.".into() } else { disabled_detail.into() },
            },
            HardwareAdapterStatus {
                adapter: "rf".into(),
                state: other_state.into(),
                detail: if self.enabled { "No production RF adapter is installed.".into() } else { disabled_detail.into() },
            },
            HardwareAdapterStatus {
                adapter: "sensors".into(),
                state: other_state.into(),
                detail: if self.enabled { "No production sensor adapter is installed.".into() } else { disabled_detail.into() },
            },
        ]
    }

    pub fn serial(&self) -> Arc<RwLock<SerialInterface>> {
        Arc::clone(&self.serial)
    }

    pub fn gpio(&self) -> Arc<RwLock<GpioInterface>> {
        Arc::clone(&self.gpio)
    }

    pub fn cellular(&self) -> Arc<RwLock<CellularInterface>> {
        Arc::clone(&self.cellular)
    }

    pub fn rf(&self) -> Arc<RwLock<RfInterface>> {
        Arc::clone(&self.rf)
    }

    pub fn sensors(&self) -> Arc<RwLock<SensorInterface>> {
        Arc::clone(&self.sensors)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unimplemented_hardware_operations_fail_instead_of_simulating_success() {
        let mut serial = SerialInterface::new("/dev/ttyJANUS_TEST");
        assert!(serial.open().await.is_err());
        assert!(serial.write(b"test").await.is_err());
        assert!(serial.read(&mut [0u8; 4]).await.is_err());

        let mut gpio = GpioInterface::new();
        assert!(gpio.set_pin(1, true).await.is_err());
        assert!(gpio.read_pin(1).await.is_err());

        assert!(CellularInterface::new().scan().await.is_err());
        assert!(RfInterface::new().scan_frequencies(100, 200).await.is_err());

        let sensors = SensorInterface::new();
        assert!(sensors.read_accelerometer().await.is_err());
        assert!(sensors.read_gps().await.is_err());
    }

    #[tokio::test]
    async fn hardware_status_never_reports_placeholder_adapters_as_ready() {
        let manager = HardwareManager::new(Some("/dev/ttyJANUS_TEST".into()), true);
        let status = manager.status().await;
        assert_eq!(status.len(), 5);
        assert_eq!(status[0].state, "unavailable");
        assert!(status.iter().skip(1).all(|adapter| adapter.state == "unsupported"));
    }
}


#[cfg(test)]
mod hardware_configuration_tests {
    use super::*;

    #[tokio::test]
    async fn disabled_hardware_integration_reports_disabled_for_every_adapter() {
        let manager = HardwareManager::new(None, false);
        manager.initialize().await.unwrap();
        let status = manager.status().await;
        assert_eq!(status.len(), 5);
        assert!(status.iter().all(|adapter| adapter.state == "disabled"));
    }
}


#[cfg(test)]
mod usb_inventory_tests {
    use super::*;
    use std::fs;

    #[test]
    fn usb_inventory_is_read_only_sorted_and_omits_serial_numbers() {
        let root = std::env::temp_dir().join(format!("janus-usb-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        for (name, vendor, product, bus, device) in [
            ("2-1", "1234", "0002", "2", "7"),
            ("1-3", "abcd", "0042", "1", "3"),
        ] {
            let directory = root.join(name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("idVendor"), vendor).unwrap();
            fs::write(directory.join("idProduct"), product).unwrap();
            fs::write(directory.join("busnum"), bus).unwrap();
            fs::write(directory.join("devnum"), device).unwrap();
            fs::write(directory.join("manufacturer"), "Example Maker").unwrap();
            fs::write(directory.join("product"), "Example Device").unwrap();
            fs::write(directory.join("serial"), "must-not-be-exposed").unwrap();
        }

        let devices = discover_usb_devices_at(&root).unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].sysfs_name, "1-3");
        assert_eq!(devices[0].vendor_id, 0xabcd);
        assert_eq!(devices[0].product_id, 0x0042);
        assert_eq!(devices[1].sysfs_name, "2-1");
        let serialized = serde_json::to_string(&devices).unwrap();
        assert!(!serialized.contains("must-not-be-exposed"));
        fs::remove_dir_all(root).unwrap();
    }
}
