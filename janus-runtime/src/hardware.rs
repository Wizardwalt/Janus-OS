//! Hardware abstraction layer for Titan device integration.
//!
//! **No production hardware driver is implemented yet.** Every operation that
//! would touch a device returns an explicit error rather than a plausible-looking
//! value. Returning `Ok(())` from a write that never happened, or `Ok((0.0, 0.0,
//! 9.8))` from an accelerometer that was never read, would let a caller report
//! unavailable hardware as working. Availability is reported from observable
//! facts only.

use anyhow::{anyhow, Result};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Honest adapter status returned to authenticated clients.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HardwareAdapterStatus {
    pub adapter: String,
    pub state: String,
    pub detail: String,
}

/// Consistent error for an adapter with no production driver.
///
/// The message names the adapter and operation so an audit trail records exactly
/// what was refused, and never implies partial success.
fn unavailable(adapter: &str, operation: &str) -> anyhow::Error {
    anyhow!(
        "hardware adapter '{}' is unsupported: no production driver is installed for '{}'",
        adapter,
        operation
    )
}

/// Hardware interface trait
pub trait HardwareInterface: Send + Sync {
    /// Get interface name
    fn name(&self) -> &str;

    /// Check if interface is available
    fn available(&self) -> bool;
}

/// Serial port interface.
///
/// The device node may exist, but no driver is implemented, so `available()`
/// reports only whether the configured path is present. It never implies that
/// reads or writes will work.
pub struct SerialInterface {
    port: String,
}

impl SerialInterface {
    pub fn new(port: impl Into<String>) -> Self {
        Self { port: port.into() }
    }

    /// The configured device path, for honest status reporting.
    pub fn port(&self) -> &str {
        &self.port
    }

    /// Whether the configured device node exists. This is a path check, not a
    /// driver or readiness check.
    pub fn device_node_exists(&self) -> bool {
        std::path::Path::new(&self.port).exists()
    }

    pub async fn open(&mut self) -> Result<()> {
        Err(unavailable("serial", "open"))
    }

    pub async fn write(&self, _data: &[u8]) -> Result<usize> {
        Err(unavailable("serial", "write"))
    }

    pub async fn read(&self, _buf: &mut [u8]) -> Result<usize> {
        Err(unavailable("serial", "read"))
    }
}

impl HardwareInterface for SerialInterface {
    fn name(&self) -> &str {
        "serial"
    }

    fn available(&self) -> bool {
        self.device_node_exists()
    }
}

/// GPIO interface
pub struct GpioInterface;

impl GpioInterface {
    pub fn new() -> Self {
        Self
    }

    pub async fn set_pin(&mut self, _pin: u32, _level: bool) -> Result<()> {
        Err(unavailable("gpio", "set_pin"))
    }

    pub async fn read_pin(&self, _pin: u32) -> Result<bool> {
        Err(unavailable("gpio", "read_pin"))
    }
}

impl Default for GpioInterface {
    fn default() -> Self {
        Self::new()
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

    /// Returns an error, not an empty list. An empty list would be
    /// indistinguishable from a successful scan that found no networks.
    pub async fn scan(&self) -> Result<Vec<CellularNetwork>> {
        Err(unavailable("cellular", "scan"))
    }
}

impl Default for CellularInterface {
    fn default() -> Self {
        Self::new()
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

    /// Returns an error, not an empty spectrum.
    pub async fn scan_frequencies(&self, _start: u32, _end: u32) -> Result<Vec<RfSignal>> {
        Err(unavailable("rf", "scan_frequencies"))
    }
}

impl Default for RfInterface {
    fn default() -> Self {
        Self::new()
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

    /// Returns an error, never a fabricated reading. A plausible value such as
    /// `(0.0, 0.0, 9.8)` would be indistinguishable from a real measurement.
    pub async fn read_accelerometer(&self) -> Result<(f64, f64, f64)> {
        Err(unavailable("sensors", "read_accelerometer"))
    }

    /// Returns an error, never `(0.0, 0.0)`, which is a real location in the
    /// Gulf of Guinea and could be mistaken for a genuine fix.
    pub async fn read_gps(&self) -> Result<(f64, f64)> {
        Err(unavailable("sensors", "read_gps"))
    }
}

impl Default for SensorInterface {
    fn default() -> Self {
        Self::new()
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

    /// Report adapter availability at startup.
    ///
    /// This never fails the daemon: absent hardware is a normal condition. It
    /// does state plainly that no driver is installed, so logs cannot be read as
    /// evidence that hardware was initialised.
    pub async fn initialize(&self) -> Result<()> {
        for status in self.status().await {
            if status.state == "available" {
                info!(
                    adapter = %status.adapter,
                    "{} (device node present; no driver implemented)",
                    status.detail
                );
            } else {
                warn!(adapter = %status.adapter, "{}", status.detail);
            }
        }
        Ok(())
    }

    /// Report only verified adapter availability; unsupported adapters never claim readiness.
    pub async fn status(&self) -> Vec<HardwareAdapterStatus> {
        let serial = self.serial.read().await;
        let serial_exists = serial.device_node_exists();
        vec![
            HardwareAdapterStatus {
                adapter: "serial".into(),
                state: if serial_exists {
                    "available"
                } else {
                    "unavailable"
                }
                .into(),
                detail: if serial_exists {
                    format!(
                        "configured path: {} (device node exists; no production driver is implemented)",
                        serial.port()
                    )
                } else {
                    format!(
                        "configured path: {} (not present; no production driver is implemented)",
                        serial.port()
                    )
                },
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
    async fn serial_write_and_read_fail_instead_of_faking_success() {
        let serial = SerialInterface::new("/dev/definitely-not-a-real-janus-port");
        assert!(serial.write(b"payload").await.is_err());
        let mut buffer = [0_u8; 8];
        assert!(serial.read(&mut buffer).await.is_err());
        // The buffer must not be populated with invented data.
        assert_eq!(buffer, [0_u8; 8]);
    }

    #[tokio::test]
    async fn serial_open_fails_because_no_driver_exists() {
        let mut serial = SerialInterface::new("/dev/definitely-not-a-real-janus-port");
        assert!(serial.open().await.is_err());
    }

    #[test]
    fn serial_availability_reflects_the_device_node_only() {
        let absent = SerialInterface::new("/dev/definitely-not-a-real-janus-port");
        assert!(!absent.available(), "a missing device node is not available");

        // A path that really exists reports available, which is a path fact and
        // not a claim that I/O works.
        let present = SerialInterface::new("/dev/null");
        if std::path::Path::new("/dev/null").exists() {
            assert!(present.available());
        }
    }

    #[tokio::test]
    async fn gpio_operations_fail() {
        let mut gpio = GpioInterface::new();
        assert!(gpio.set_pin(1, true).await.is_err());
        assert!(gpio.read_pin(1).await.is_err());
        assert!(!gpio.available());
    }

    #[tokio::test]
    async fn cellular_scan_errors_rather_than_returning_an_empty_scan() {
        let cellular = CellularInterface::new();
        assert!(cellular.scan().await.is_err());
        assert!(!cellular.available());
    }

    #[tokio::test]
    async fn rf_scan_errors_rather_than_returning_an_empty_spectrum() {
        let rf = RfInterface::new();
        assert!(rf.scan_frequencies(400_000_000, 500_000_000).await.is_err());
        assert!(!rf.available());
    }

    #[tokio::test]
    async fn sensors_never_return_fabricated_readings() {
        let sensors = SensorInterface::new();
        assert!(sensors.read_accelerometer().await.is_err());
        assert!(sensors.read_gps().await.is_err());
        assert!(!sensors.available());
    }

    #[tokio::test]
    async fn status_never_claims_an_unimplemented_adapter_is_ready() {
        let manager = HardwareManager::new(Some(
            "/dev/definitely-not-a-real-janus-port".to_string(),
        ));
        let statuses = manager.status().await;
        assert_eq!(statuses.len(), 5);
        for status in &statuses {
            assert_ne!(
                status.state, "ready",
                "{} must not report ready",
                status.adapter
            );
            if status.adapter != "serial" {
                assert_eq!(status.state, "unsupported");
            } else {
                assert_eq!(status.state, "unavailable");
            }
            assert!(
                status.detail.to_lowercase().contains("no production")
                    || status.detail.to_lowercase().contains("not present"),
                "{} detail must explain the absence of a driver: {}",
                status.adapter,
                status.detail
            );
        }
    }

    #[tokio::test]
    async fn initialize_succeeds_without_hardware_but_claims_nothing() {
        let manager = HardwareManager::new(Some(
            "/dev/definitely-not-a-real-janus-port".to_string(),
        ));
        // Absent hardware is normal and must not abort startup.
        assert!(manager.initialize().await.is_ok());
        // ...and it must not make any adapter appear usable.
        assert!(manager.status().await.iter().all(|s| s.state != "ready"));
    }
}
