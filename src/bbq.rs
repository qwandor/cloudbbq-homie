use crate::config::{get_mqtt_options, Config, DeviceConfig};
use bluez_async::{BluetoothSession, DeviceInfo, MacAddress};
use cloudbbq::{BBQDevice, RealTimeData, SettingResult};
use eyre::{bail, Report};
use futures::select;
use futures::stream::StreamExt;
use homie_device::{HomieDevice, Node, Property};
use rumqttc::ClientConfig;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

const NODE_ID_BATTERY: &str = "battery";
const PROPERTY_ID_VOLTAGE: &str = "voltage";
const PROPERTY_ID_PERCENTAGE: &str = "percentage";
const NODE_ID_PROBE_PREFIX: &str = "probe";
const PROPERTY_ID_TEMPERATURE: &str = "temperature";
const PROPERTY_ID_TARGET_TEMPERATURE: &str = "target";
const PROPERTY_ID_TARGET_MODE: &str = "mode";
const TARGET_MODE_NONE: &str = "None";
const TARGET_MODE_SINGLE: &str = "Target";
const TARGET_MODES: [&str; 2] = [TARGET_MODE_NONE, TARGET_MODE_SINGLE];

#[derive(Debug)]
pub struct BBQ {
    mac_address: MacAddress,
    config: Config,
    device_config: DeviceConfig,
    name: String,
    device: BBQDevice,
}

impl BBQ {
    /// Attempt to connect to the given Barbecue thermometer device and authenticate with it.
    pub async fn connect(
        session: &BluetoothSession,
        device: DeviceInfo,
        config: Config,
    ) -> Result<BBQ, Report> {
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
        let name = device_config.name.clone().unwrap_or(device.name.unwrap());
        Ok(BBQ {
            mac_address: device.mac_address,
            config,
            device_config,
            name,
            device: connected_device,
        })
    }

    /// Create a Homie device for the Barbecue thermometer, and keep publishing updates.
    pub async fn run(self, tls_client_config: Option<Arc<ClientConfig>>) -> Result<(), Report> {
        let device_id_suffix = self.mac_address.to_string().replace(":", "");
        let device_base = format!(
            "{}/{}-{}",
            self.config.homie.prefix, self.config.homie.device_id_prefix, device_id_suffix
        );
        let mqtt_options =
            get_mqtt_options(&self.config.mqtt, &device_id_suffix, tls_client_config);
        let mut homie_builder = HomieDevice::builder(&device_base, &self.name, mqtt_options);
        homie_builder.set_firmware(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        let device_clone = self.device.clone();
        let target_state = Arc::new(Mutex::new(TargetState::default()));
        homie_builder.set_update_callback(move |node_id, property_id, value| {
            let device_clone = device_clone.clone();
            let target_state = target_state.clone();
            async {
                Self::handle_update(device_clone, target_state, node_id, property_id, value).await
            }
        });
        let (mut homie, homie_handle) = homie_builder.spawn().await?;
        homie.ready().await?;
        homie
            .add_node(Node::new(
                NODE_ID_BATTERY,
                "Battery",
                "Battery level",
                vec![
                    Property::integer(PROPERTY_ID_VOLTAGE, "Voltage", false, None, None),
                    Property::integer(PROPERTY_ID_PERCENTAGE, "Percentage", false, Some("%"), None),
                ],
            ))
            .await?;

        let mut setting_results = self.device.setting_results().await?.fuse();
        let mut real_time_data = self.device.real_time().await?.fuse();
        self.device.enable_real_time_data(true).await?;
        // Request an initial battery level reading.
        self.device.request_battery_level().await?;

        loop {
            select! {
                data = real_time_data.select_next_some() => self.handle_realtime_data(data, &mut homie).await?,
                result = setting_results.select_next_some() => self.handle_setting_result(result, &mut homie).await?,
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
        if let Some(probe_index) = probe_id_to_index(&node_id) {
            let target = {
                let state = &mut *target_state.lock().unwrap();
                let target = state.targets.entry(probe_index).or_default();
                match property_id.as_ref() {
                    PROPERTY_ID_TARGET_TEMPERATURE => {
                        target.temperature = value.parse().ok()?;
                    }
                    PROPERTY_ID_TARGET_MODE => {
                        target.mode = value.parse().ok()?;
                    }
                    _ => return None,
                };
                target.clone()
            };
            match target.mode {
                TargetMode::None => {
                    //TODO: Remove target temperature.
                }
                TargetMode::Single => {
                    device
                        .set_target_temp(probe_index, target.temperature)
                        .await
                        .ok()?;
                }
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
            _ => {}
        }
        Ok(())
    }

    fn node_for_probe(&self, node_id: &str, probe_index: usize) -> Node {
        let default_probe_name = format!("Probe {}", probe_index + 1);
        let probe_name = self
            .device_config
            .probe_names
            .get(probe_index)
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
                    Some("ºC"),
                    None,
                ),
                Property::float(
                    PROPERTY_ID_TARGET_TEMPERATURE,
                    "Target temperature",
                    true,
                    Some("ºC"),
                    None,
                ),
                Property::enumeration(
                    PROPERTY_ID_TARGET_MODE,
                    "Target mode",
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
                    homie
                        .add_node(self.node_for_probe(&node_id, probe_index))
                        .await?;
                    // Send default values for settable properties.
                    homie
                        .publish_value(&node_id, PROPERTY_ID_TARGET_MODE, TARGET_MODE_NONE)
                        .await?;
                    homie
                        .publish_value(&node_id, PROPERTY_ID_TARGET_TEMPERATURE, 0.0)
                        .await?;
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
}

/// The target temperatures set for each probe.
#[derive(Debug, Default)]
struct TargetState {
    /// Map from probe index to target settings.
    targets: HashMap<u8, Target>,
}

/// The target mode and temperature for a single probe.
#[derive(Clone, Default, Debug)]
struct Target {
    mode: TargetMode,
    temperature: f32,
}

#[derive(Copy, Clone, Debug)]
enum TargetMode {
    None,
    Single,
}

impl FromStr for TargetMode {
    type Err = Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            TARGET_MODE_NONE => Ok(Self::None),
            TARGET_MODE_SINGLE => Ok(Self::Single),
            _ => bail!("Invalid target mode {}", s),
        }
    }
}

impl Default for TargetMode {
    fn default() -> Self {
        TargetMode::None
    }
}

fn probe_id_to_index(probe_id: &str) -> Option<u8> {
    probe_id.strip_prefix(NODE_ID_PROBE_PREFIX)?.parse().ok()
}
