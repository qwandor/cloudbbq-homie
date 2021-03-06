mod config;

use crate::config::{get_mqtt_options, get_tls_client_config, Config};
use bluez_async::{BluetoothSession, DeviceInfo, MacAddress};
use cloudbbq::{find_devices, BBQDevice, RealTimeData, SettingResult};
use eyre::{bail, Report};
use futures::future::try_join_all;
use futures::stream::StreamExt;
use futures::{select, TryFutureExt};
use homie_device::{HomieDevice, Node, Property};
use rumqttc::ClientConfig;
use std::sync::Arc;
use std::time::Duration;
use tokio::{task, time, try_join};

const SCAN_DURATION: Duration = Duration::from_secs(5);

const NODE_ID_BATTERY: &str = "battery";
const PROPERTY_ID_VOLTAGE: &str = "voltage";
const PROPERTY_ID_PERCENTAGE: &str = "percentage";
const NODE_ID_PROBE_PREFIX: &str = "probe";
const PROPERTY_ID_TEMPERATURE: &str = "temperature";

#[tokio::main]
async fn main() -> Result<(), Report> {
    stable_eyre::install()?;
    pretty_env_logger::init();
    color_backtrace::install();

    let config = Config::from_file()?;
    let tls_client_config = get_tls_client_config(&config.mqtt);

    // Connect a Bluetooth session.
    let (dbus_handle, session) = BluetoothSession::new().await?;

    let bbq_handle = run_system(&config, tls_client_config, &session);

    // Poll everything to completion, until the first one bombs out.
    let res: Result<_, Report> = try_join! {
        // If this ever finishes, we lost connection to D-Bus.
        dbus_handle.err_into(),
        bbq_handle.err_into(),
    };
    res?;

    Ok(())
}

async fn run_system(
    config: &Config,
    tls_client_config: Option<Arc<ClientConfig>>,
    session: &BluetoothSession,
) -> Result<(), Report> {
    log::info!("Starting discovery");
    session.start_discovery().await?;
    time::sleep(SCAN_DURATION).await;
    let devices = find_devices(session).await?;
    if devices.is_empty() {
        bail!("No devices found");
    }

    let mut join_handles = vec![];
    for device in devices {
        let bbq = BBQ::connect(session, device).await?;
        let handle = task::spawn(bbq.run(config.to_owned(), tls_client_config.clone()));
        join_handles.push(handle);
    }
    try_join_all(join_handles).await?;

    Ok(())
}

#[derive(Debug)]
struct BBQ {
    mac_address: MacAddress,
    name: String,
    device: BBQDevice,
}

impl BBQ {
    /// Attempt to connect to the given Barbecue thermometer device and authenticate with it.
    async fn connect(session: &BluetoothSession, device: DeviceInfo) -> Result<BBQ, Report> {
        log::info!("Connecting to {:?}...", device);
        session.connect(&device.id).await?;
        let connected_device = BBQDevice::new(session.clone(), device.id).await?;
        log::info!("Authenticating...");
        connected_device.authenticate().await?;
        log::info!("Authenticated.");
        Ok(BBQ {
            mac_address: device.mac_address,
            name: device.name.unwrap(),
            device: connected_device,
        })
    }

    /// Create a Homie device for the Barbecue thermometer, and keep publishing updates.
    async fn run(
        self,
        config: Config,
        tls_client_config: Option<Arc<ClientConfig>>,
    ) -> Result<(), Report> {
        let device_id_suffix = self.mac_address.to_string().replace(":", "");
        let device_base = format!(
            "{}/{}-{}",
            config.homie.prefix, config.homie.device_id_prefix, device_id_suffix
        );
        let mqtt_options = get_mqtt_options(&config.mqtt, &device_id_suffix, tls_client_config);
        let mut homie_builder = HomieDevice::builder(&device_base, &self.name, mqtt_options);
        homie_builder.set_firmware(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
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
                        .add_node(Node::new(
                            &node_id,
                            &format!("Probe {}", probe_index + 1),
                            "Temperature probe",
                            vec![Property::float(
                                PROPERTY_ID_TEMPERATURE,
                                "Temperature",
                                false,
                                Some("ºC"),
                                None,
                            )],
                        ))
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
