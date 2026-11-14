//! Hardware abstraction layer for Titan device integration.

use anyhow::Result;
use serde::Serialize;
use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
    time::Duration,
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
