// Copyright 2021 the cloudbbq-homie authors.
// This project is dual-licensed under Apache 2.0 and MIT terms.
// See LICENSE-APACHE and LICENSE-MIT for details.

use crate::config::{get_mqtt_options, Config, DeviceConfig};
use bluez_async::{BluetoothSession, DeviceInfo, MacAddress};
use cloudbbq::{BBQDevice, RealTimeData, SettingResult, TemperatureUnit};
use eyre::{bail, Report, WrapErr};
use futures::stream::StreamExt;
use futures::{select, FutureExt};
use homie_device::{HomieDevice, Node, Property};
use rustls::ClientConfig;
use std::collections::HashMap;
use std::fmt::{self, Debug, Display, Formatter};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

const NODE_ID_BATTERY: &str = "battery";
const PROPERTY_ID_VOLTAGE: &str = "voltage";
const PROPERTY_ID_PERCENTAGE: &str = "percentage";

const NODE_ID_SETTINGS: &str = "settings";
const PROPERTY_ID_DISPLAY_UNIT: &str = "unit";
const PROPERTY_ID_ALARM: &str = "alarm";
const DISPLAY_UNIT_CELCIUS: &str = "ºC";
const DISPLAY_UNIT_FAHRENHEIT: &str = "ºF";
const DISPLAY_UNITS: [&str; 2] = [DISPLAY_UNIT_CELCIUS, DISPLAY_UNIT_FAHRENHEIT];

const NODE_ID_PROBE_PREFIX: &str = "probe";
const PROPERTY_ID_TEMPERATURE: &str = "temperature";
const PROPERTY_ID_TARGET_TEMPERATURE_MIN: &str = "target_min";
const PROPERTY_ID_TARGET_TEMPERATURE_MAX: &str = "target_max";
const PROPERTY_ID_TARGET_MODE: &str = "mode";
const TARGET_MODE_NONE: &str = "None";
const TARGET_MODE_SINGLE: &str = "Maximum only";
const TARGET_MODE_RANGE: &str = "Range";
const TARGET_MODES: [&str; 3] = [TARGET_MODE_NONE, TARGET_MODE_SINGLE, TARGET_MODE_RANGE];

#[derive(Debug)]
pub struct Bbq {
    mac_address: MacAddress,
    config: Config,
    device_config: DeviceConfig,
    name: String,
    device: BBQDevice,
    target_state: Arc<Mutex<TargetState>>,
}

impl Bbq {
    /// Attempt to connect to the given Barbecue thermometer device and authenticate with it.
    pub async fn connect(
        session: &BluetoothSession,
        device: DeviceInfo,
        config: Config,
    ) -> Result<Bbq, Report> {
        log::info!("Connecting to {:?}...", device);
        session.connect(&device.id).await?;
        let connected_device = BBQDevice::new(session.clone(), device.id).await?;
        log::info!("Authenticating...");
        connected_device.authenticate().await?;
        log::info!("Authenticated.");

        let device_config = config
            .devices
            .get(&device.mac_address)
            .cloned()
            .unwrap_or_default();
        // Use the configured name if there is one, otherwise the Bluetooth device name.
        let bluetooth_device_name = device.name.unwrap();
        let name = device_config.name.clone().unwrap_or(bluetooth_device_name);
        Ok(Bbq {
            mac_address: device.mac_address,
            config,
            device_config,
            name,
            device: connected_device,
            target_state: Arc::new(Mutex::new(TargetState::default())),
        })
    }

    /// Create a Homie device for the Barbecue thermometer, and keep publishing updates.
    pub async fn run(self, tls_client_config: Option<Arc<ClientConfig>>) -> Result<(), Report> {
        let device_id_suffix = self.mac_address.to_string().replace(':', "");
        let device_base = format!(
            "{}/{}-{}",
            self.config.homie.prefix, self.config.homie.device_id_prefix, device_id_suffix
        );
        let mqtt_options =
            get_mqtt_options(&self.config.mqtt, &device_id_suffix, tls_client_config);
        let mut homie_builder = HomieDevice::builder(&device_base, &self.name, mqtt_options);
        homie_builder.set_firmware(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        let device_clone = self.device.clone();
        let target_state = self.target_state.clone();
        homie_builder.set_update_callback(move |node_id, property_id, value| {
            let device_clone = device_clone.clone();
            let target_state = target_state.clone();
            async {
                Self::handle_update(device_clone, target_state, node_id, property_id, value).await
            }
        });
        let (mut homie, homie_handle) = homie_builder.spawn().await?;
        homie.ready().await?;

        // Add nodes other than probes.
        homie
            .add_node(Node::new(
                NODE_ID_BATTERY,
                "Battery",
                "Battery level",
                vec![
                    Property::integer(PROPERTY_ID_VOLTAGE, "Voltage", false, true, None, None),
                    Property::integer(
                        PROPERTY_ID_PERCENTAGE,
                        "Percentage",
                        false,
                        true,
                        Some("%"),
                        None,
                    ),
                ],
            ))
            .await?;
        homie
            .add_node(Node::new(
                NODE_ID_SETTINGS,
                "Settings",
                "Settings",
                vec![
                    Property::enumeration(
                        PROPERTY_ID_DISPLAY_UNIT,
                        "Unit",
                        true,
                        true,
                        None,
                        &DISPLAY_UNITS,
                    ),
                    Property::boolean(PROPERTY_ID_ALARM, "Alarm", true, false, None),
                ],
            ))
            .await?;
        // Default to Celcius.
        self.device
            .set_temperature_unit(TemperatureUnit::Celcius)
            .await?;
        homie
            .publish_value(
                NODE_ID_SETTINGS,
                PROPERTY_ID_DISPLAY_UNIT,
                DISPLAY_UNIT_CELCIUS,
            )
            .await?;

        let mut setting_results = self.device.setting_results().await?.fuse();
        let mut real_time_data = self.device.real_time().await?.fuse();
        self.device.enable_real_time_data(true).await?;
        // Request an initial battery level reading.
        self.device.request_battery_level().await?;

        let mut homie_handle = homie_handle.fuse();

        loop {
            select! {
                data = real_time_data.select_next_some() => self.handle_realtime_data(data, &mut homie).await?,
                result = setting_results.select_next_some() => self.handle_setting_result(result, &mut homie).await?,
                homie_result = homie_handle => return homie_result.wrap_err("Homie error"),
                complete => break,
            };
        }

        Ok(())
    }

    async fn handle_update(
        device: BBQDevice,
        target_state: Arc<Mutex<TargetState>>,
        node_id: String,
        property_id: String,
        value: String,
    ) -> Option<String> {
        log::trace!("{}/{} = {}", node_id, property_id, value);
        if node_id == NODE_ID_SETTINGS && property_id == PROPERTY_ID_DISPLAY_UNIT {
            let unit = parse_display_unit(&value)?;
            if let Err(e) = device.set_temperature_unit(unit).await {
                log::error!("Failed to set temperature unit: {}", e);
                return None;
            }
            Some(value)
        } else if node_id == NODE_ID_SETTINGS && property_id == PROPERTY_ID_ALARM {
            let state: bool = value.parse().ok()?;
            if !state {
                if let Err(e) = device.silence_alarm().await {
                    log::error!("Failed to silence alarm: {}", e);
                    return None;
                }
                Some(value)
            } else {
                None
            }
        } else if let Some(probe_index) = probe_id_to_index(&node_id) {
            let target = {
                let state = &mut *target_state.lock().unwrap();
                let target = state.target(probe_index);
                match property_id.as_ref() {
                    PROPERTY_ID_TARGET_TEMPERATURE_MIN => {
                        target.temperature_min = value.parse().ok()?;
                    }
                    PROPERTY_ID_TARGET_TEMPERATURE_MAX => {
                        target.temperature_max = value.parse().ok()?;
                    }
                    PROPERTY_ID_TARGET_MODE => {
                        target.mode = value.parse().ok()?;
                    }
                    _ => return None,
                };
                target.clone()
            };
            if let Err(e) = set_target(&device, probe_index, &target).await {
                log::error!("Failed to set target temperature: {}", e);
                return None;
            }
            Some(value)
        } else {
            None
        }
    }

    async fn handle_setting_result(
        &self,
        result: SettingResult,
        homie: &mut HomieDevice,
    ) -> Result<(), Report> {
        log::trace!("Setting result: {:?}", result);
        match result {
            SettingResult::BatteryLevel {
                current_voltage,
                max_voltage,
            } => {
                let percentage = current_voltage as u32 * 100 / max_voltage as u32;
                homie
                    .publish_value(NODE_ID_BATTERY, PROPERTY_ID_VOLTAGE, current_voltage)
                    .await?;
                homie
                    .publish_value(NODE_ID_BATTERY, PROPERTY_ID_PERCENTAGE, percentage)
                    .await?;
            }
            SettingResult::SilencePressed => {
                homie
                    .publish_nonretained_value(NODE_ID_SETTINGS, PROPERTY_ID_ALARM, false)
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }

    fn node_for_probe(&self, node_id: &str, probe_index: u8) -> Node {
        let default_probe_name = format!("Probe {}", probe_index + 1);
        let probe_name = self
            .device_config
            .probe_names
            .get(probe_index as usize)
            .unwrap_or(&default_probe_name);
        Node::new(
            node_id,
            probe_name,
            "Temperature probe",
            vec![
                Property::float(
                    PROPERTY_ID_TEMPERATURE,
                    "Temperature",
                    false,
                    true,
                    Some("ºC"),
                    None,
                ),
                Property::float(
                    PROPERTY_ID_TARGET_TEMPERATURE_MIN,
                    "Minimum temperature",
                    true,
                    true,
                    Some("ºC"),
                    None,
                ),
                Property::float(
                    PROPERTY_ID_TARGET_TEMPERATURE_MAX,
                    "Target/maximum temperature",
                    true,
                    true,
                    Some("ºC"),
                    None,
                ),
                Property::enumeration(
                    PROPERTY_ID_TARGET_MODE,
                    "Target mode",
                    true,
                    true,
                    None,
                    &TARGET_MODES,
                ),
            ],
        )
    }

    async fn handle_realtime_data(
        &self,
        data: RealTimeData,
        homie: &mut HomieDevice,
    ) -> Result<(), Report> {
        log::trace!("Realtime data: {:?}", data);
        for (probe_index, temperature) in data.probe_temperatures.into_iter().enumerate() {
            let node_id = format!("{}{}", NODE_ID_PROBE_PREFIX, probe_index);
            let exists = homie.has_node(&node_id);
            if let Some(temperature) = temperature {
                if !exists {
                    self.add_probe(homie, probe_index as u8, &node_id).await?;
                }
                homie
                    .publish_value(&node_id, PROPERTY_ID_TEMPERATURE, temperature)
                    .await?;
            } else if exists {
                homie.remove_node(&node_id).await?;
            }
        }
        Ok(())
    }

    async fn add_probe(
        &self,
        homie: &mut HomieDevice,
        probe_index: u8,
        node_id: &str,
    ) -> Result<(), Report> {
        homie
            .add_node(self.node_for_probe(node_id, probe_index))
            .await?;

        // Restore the target temperature to its previous value, or none.
        let target = self
            .target_state
            .lock()
            .unwrap()
            .target(probe_index)
            .clone();
        set_target(&self.device, probe_index, &target).await?;
        homie
            .publish_value(node_id, PROPERTY_ID_TARGET_MODE, target.mode)
            .await?;
        homie
            .publish_value(
                node_id,
                PROPERTY_ID_TARGET_TEMPERATURE_MIN,
                target.temperature_min,
            )
            .await?;
        homie
            .publish_value(
                node_id,
                PROPERTY_ID_TARGET_TEMPERATURE_MAX,
                target.temperature_max,
            )
            .await?;

        Ok(())
    }
}

async fn set_target(device: &BBQDevice, probe_index: u8, target: &Target) -> Result<(), Report> {
    match target.mode {
        TargetMode::None => device.remove_target(probe_index).await,
        TargetMode::Single => {
            device
                .set_target_temp(probe_index, target.temperature_max)
                .await
        }
        TargetMode::Range => {
            device
                .set_target_range(probe_index, target.temperature_min..target.temperature_max)
                .await
        }
    }
    .wrap_err("Failed to set target temperature")
}

/// The target temperatures set for each probe.
#[derive(Debug, Default)]
struct TargetState {
    /// Map from probe index to target settings.
    targets: HashMap<u8, Target>,
}

impl TargetState {
    fn target(&mut self, probe_index: u8) -> &mut Target {
        self.targets.entry(probe_index).or_default()
    }
}

/// The target mode and temperature for a single probe.
#[derive(Clone, Default, Debug)]
struct Target {
    mode: TargetMode,
    temperature_min: f32,
    temperature_max: f32,
}

#[derive(Copy, Clone, Debug, Default)]
enum TargetMode {
    #[default]
    None,
    Single,
    Range,
}

impl FromStr for TargetMode {
    type Err = Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            TARGET_MODE_NONE => Ok(Self::None),
            TARGET_MODE_SINGLE => Ok(Self::Single),
            TARGET_MODE_RANGE => Ok(Self::Range),
            _ => bail!("Invalid target mode {}", s),
        }
    }
}

impl TargetMode {
    fn as_str(&self) -> &'static str {
        match self {
            Self::None => TARGET_MODE_NONE,
            Self::Single => TARGET_MODE_SINGLE,
            Self::Range => TARGET_MODE_RANGE,
        }
    }
}

impl Display for TargetMode {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn probe_id_to_index(probe_id: &str) -> Option<u8> {
    probe_id.strip_prefix(NODE_ID_PROBE_PREFIX)?.parse().ok()
}

fn parse_display_unit(value: &str) -> Option<TemperatureUnit> {
    match value {
        DISPLAY_UNIT_CELCIUS => Some(TemperatureUnit::Celcius),
        DISPLAY_UNIT_FAHRENHEIT => Some(TemperatureUnit::Fahrenheit),
        _ => None,
    }
}
