//! Hardware abstraction layer for Titan device integration.

use anyhow::Result;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

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
    available: bool,
}

impl SerialInterface {
    pub fn new(port: impl Into<String>) -> Self {
        Self {
            port: port.into(),
            available: false,
        }
    }

    pub async fn open(&mut self) -> Result<()> {
        anyhow::bail!(
            "serial adapter is not implemented; refusing to report a simulated open for {}",
            self.port
        )
    }

    pub async fn write(&self, _data: &[u8]) -> Result<usize> {
        anyhow::bail!("serial adapter is not implemented; no bytes were written")
    }

    pub async fn read(&self, _buf: &mut [u8]) -> Result<usize> {
        anyhow::bail!("serial adapter is not implemented; no device data was read")
    }
}

impl HardwareInterface for SerialInterface {
    fn name(&self) -> &str {
        "serial"
    }

    fn available(&self) -> bool {
        self.available
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
    serial: Arc<RwLock<SerialInterface>>,
    gpio: Arc<RwLock<GpioInterface>>,
    cellular: Arc<RwLock<CellularInterface>>,
    rf: Arc<RwLock<RfInterface>>,
    sensors: Arc<RwLock<SensorInterface>>,
}

impl HardwareManager {
    pub fn new(serial_port: Option<String>) -> Self {
        let port = serial_port.unwrap_or_else(|| "/dev/ttyUSB0".to_string());
        Self {
            serial: Arc::new(RwLock::new(SerialInterface::new(port))),
            gpio: Arc::new(RwLock::new(GpioInterface::new())),
            cellular: Arc::new(RwLock::new(CellularInterface::new())),
            rf: Arc::new(RwLock::new(RfInterface::new())),
            sensors: Arc::new(RwLock::new(SensorInterface::new())),
        }
    }

    pub async fn initialize(&self) -> Result<()> {
        info!("Initializing hardware manager");
        // Would initialize each interface
        Ok(())
    }

    /// Report only verified adapter availability; unsupported adapters never claim readiness.
    pub async fn status(&self) -> Vec<HardwareAdapterStatus> {
        let serial = self.serial.read().await;
        vec![
            HardwareAdapterStatus {
                adapter: "serial".into(),
                state: "unsupported".into(),
                detail: format!(
                    "configured path: {}; a production serial driver is not implemented",
                    serial.port
                ),
            },
            HardwareAdapterStatus {
                adapter: "gpio".into(),
                state: "unsupported".into(),
                detail: "No production GPIO driver is installed.".into(),
            },
            HardwareAdapterStatus {
                adapter: "cellular".into(),
                state: "unsupported".into(),
                detail: "No production cellular adapter is installed.".into(),
            },
            HardwareAdapterStatus {
                adapter: "rf".into(),
                state: "unsupported".into(),
                detail: "No production RF adapter is installed.".into(),
            },
            HardwareAdapterStatus {
                adapter: "sensors".into(),
                state: "unsupported".into(),
                detail: "No production sensor adapter is installed.".into(),
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
        let manager = HardwareManager::new(Some("/dev/ttyJANUS_TEST".into()));
        let status = manager.status().await;
        assert_eq!(status.len(), 5);
        assert!(status.iter().all(|adapter| adapter.state == "unsupported"));
    }
}
